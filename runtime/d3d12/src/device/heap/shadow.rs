//! Anonymous shadow copies when the host rejects section-backed host-memory imports.
use super::{ImportedHeap, trace_import_failure};
use crate::commands::page_mapping::ImportPin;
use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use windows::Win32::Foundation::{
    CloseHandle, E_INVALIDARG, E_OUTOFMEMORY, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_COPY, D3D12_COMMAND_QUEUE_DESC, D3D12_FENCE_FLAG_NONE,
    D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_READBACK, D3D12_RANGE,
    D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER,
    D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_TEXTURE_LAYOUT_ROW_MAJOR, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
    ID3D12Device, ID3D12Device3, ID3D12Device13, ID3D12Fence, ID3D12GraphicsCommandList,
    ID3D12Heap, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEM_COMMIT, MEM_MAPPED, MEM_RELEASE, MEM_RESERVE,
    MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_READWRITE, UnmapViewOfFile, VirtualAlloc, VirtualFree, VirtualQuery,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{Interface, Result};

const MAX_SHADOW_BYTES: usize = 1 << 30;
const PAGE_STEP: usize = 4096;
const CHUNK_BYTES: usize = 64;

struct ShadowState {
    baseline: Vec<u8>,
    retired: bool,
}

/// Private anonymous host copy of committed section-backed guest pages.
/// On drivers such as RADV that restrict `VK_EXT_external_memory_host` to
/// anonymous userptr pages, `OpenExistingHeapFromAddress*` imports this buffer
/// while CPU modifications in non-retired pages are mirrored before submit.
pub(super) struct ShadowPages {
    guest_base: u64,
    shadow_base: usize,
    size: usize,
    pin_live: Arc<AtomicBool>,
    state: Mutex<ShadowState>,
}

struct AllocGuard(usize);
impl Drop for AllocGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            let _ = unsafe { VirtualFree(self.0 as _, 0, MEM_RELEASE) };
        }
    }
}

impl Drop for ShadowPages {
    fn drop(&mut self) {
        if self.shadow_base != 0 {
            // SAFETY: `shadow_base` was allocated by `VirtualAlloc(MEM_RESERVE | MEM_COMMIT)`
            // and outlives `ImportedHeap::native` (declared earlier in `ImportedHeap`).
            let _ = unsafe { VirtualFree(self.shadow_base as _, 0, MEM_RELEASE) };
        }
    }
}

impl ShadowPages {
    fn new(guest_base: u64, size: usize, pin_live: Arc<AtomicBool>) -> Result<Arc<Self>> {
        if size == 0 || size > MAX_SHADOW_BYTES {
            return Err(E_INVALIDARG.into());
        }
        let mut baseline = Vec::new();
        baseline
            .try_reserve_exact(size)
            .map_err(|_| E_OUTOFMEMORY)?;
        // SAFETY: `None` base asks the OS for an anonymous private region of `size` bytes.
        let shadow_ptr = unsafe { VirtualAlloc(None, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        if shadow_ptr.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        let mut guard = AllocGuard(shadow_ptr.addr());
        if !is_guest_readable(guest_base, size) {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: `is_guest_readable` confirmed `guest_base..guest_base + size` is committed and
        // readable, and `shadow_ptr` holds `size` writable bytes.
        let guest = unsafe { core::slice::from_raw_parts(guest_base as *const u8, size) };
        baseline.extend_from_slice(guest);
        unsafe { core::ptr::copy_nonoverlapping(baseline.as_ptr(), shadow_ptr.cast(), size) };
        let shadow_base = guard.0;
        guard.0 = 0;
        Ok(Arc::new(Self {
            guest_base,
            shadow_base,
            size,
            pin_live,
            state: Mutex::new(ShadowState {
                baseline,
                retired: false,
            }),
        }))
    }

    fn ptr(&self) -> *const c_void {
        self.shadow_base as _
    }

    fn is_retired(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).retired
    }

    /// Copy CPU-modified 64-byte chunks into the anonymous shadow buffer using
    /// a fast 4 KiB page compare first. Returns `false` once retired by a remap.
    fn sync_cpu_writes(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired {
            return false;
        }
        if !self.pin_live.load(Ordering::Acquire) {
            state.retired = true;
            return false;
        }
        if !is_guest_readable(self.guest_base, self.size) {
            return true;
        }
        // SAFETY: `is_guest_readable` checked commit + protection on `guest_base..+size`,
        // and `shadow_base` owns `size` RW bytes guarded by `self.state`.
        let guest = unsafe { core::slice::from_raw_parts(self.guest_base as *const u8, self.size) };
        let shadow = unsafe { core::slice::from_raw_parts_mut(self.shadow_base as *mut u8, self.size) };
        diff_copy_chunks(guest, &mut state.baseline, shadow);
        true
    }
}

pub(super) fn is_userptr_refusal(error: &windows::core::Error) -> bool {
    matches!(error.code(), E_OUTOFMEMORY | E_INVALIDARG)
}

/// True when `base..base+size` is fully committed `PAGE_READWRITE` and contains
/// at least one section-backed (`MEM_MAPPED`) sub-region.
pub(super) fn contains_mapped_pages(base: u64, size: u64) -> bool {
    let Some(end) = base.checked_add(size) else {
        return false;
    };
    if size == 0 {
        return false;
    }
    let mut cursor = base;
    let mut saw_mapped = false;
    while cursor < end {
        let mut region = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: `VirtualQuery` only inspects virtual memory attributes.
        if unsafe { VirtualQuery(Some(cursor as _), &mut region, size_of_val(&region)) } == 0
            || region.State != MEM_COMMIT
            || region.Protect != PAGE_READWRITE
        {
            return false;
        }
        if region.Type == MEM_MAPPED {
            saw_mapped = true;
        }
        let Some(next) = (region.BaseAddress as u64).checked_add(region.RegionSize as u64) else {
            return false;
        };
        if next <= cursor {
            return false;
        }
        cursor = next;
    }
    saw_mapped
}

fn is_guest_readable(address: u64, size: usize) -> bool {
    let Some(end) = address.checked_add(size as u64) else {
        return false;
    };
    if size == 0 {
        return true;
    }
    let mut cursor = address;
    while cursor < end {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: `VirtualQuery` only inspects the process memory map.
        let queried = unsafe { VirtualQuery(Some(cursor as _), &mut info, size_of_val(&info)) };
        let protect = info.Protect.0;
        if queried == 0
            || info.State != MEM_COMMIT
            || protect & PAGE_GUARD.0 != 0
            || protect & 0xff == PAGE_NOACCESS.0
        {
            return false;
        }
        let Some(next) = (info.BaseAddress as u64).checked_add(info.RegionSize as u64) else {
            return false;
        };
        if next <= cursor {
            return false;
        }
        cursor = next;
    }
    true
}

/// Skip unchanged 4 KiB pages, then sync only 64-byte chunks where CPU memory
/// diverged from `baseline`, leaving any GPU-written chunks in `shadow` intact.
fn diff_copy_chunks(guest: &[u8], baseline: &mut [u8], shadow: &mut [u8]) -> usize {
    let mut copied = 0usize;
    for ((gp, bp), sp) in guest
        .chunks(PAGE_STEP)
        .zip(baseline.chunks_mut(PAGE_STEP))
        .zip(shadow.chunks_mut(PAGE_STEP))
    {
        if gp == &*bp {
            continue;
        }
        for ((g, b), s) in gp
            .chunks(CHUNK_BYTES)
            .zip(bp.chunks_mut(CHUNK_BYTES))
            .zip(sp.chunks_mut(CHUNK_BYTES))
        {
            if g != &*b {
                s.copy_from_slice(g);
                b.copy_from_slice(g);
                copied += g.len();
            }
        }
    }
    copied
}

static SHADOWS: LazyLock<Mutex<Vec<Weak<ShadowPages>>>> = LazyLock::new(Default::default);
static SHADOW_COUNT: AtomicU32 = AtomicU32::new(0);

fn register(shadow: &Arc<ShadowPages>) {
    let mut shadows = SHADOWS.lock().unwrap_or_else(|e| e.into_inner());
    shadows.retain(|weak| weak.strong_count() != 0);
    shadows.push(Arc::downgrade(shadow));
    SHADOW_COUNT.store(shadows.len().min(u32::MAX as usize) as u32, Ordering::Release);
}

/// Synchronize CPU writes into all active anonymous shadow heaps before GPU
/// command-list submission. Zero-lock fast path when no shadows exist.
pub(crate) fn sync_shadow_imports() {
    if SHADOW_COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    let sync_live = |live: &mut Vec<Arc<ShadowPages>>| {
        live.clear();
        {
            let mut shadows = SHADOWS.lock().unwrap_or_else(|e| e.into_inner());
            shadows.retain(|weak| weak.strong_count() != 0);
            live.extend(shadows.iter().filter_map(Weak::upgrade));
            SHADOW_COUNT.store(shadows.len().min(u32::MAX as usize) as u32, Ordering::Release);
        }
        let mut any_retired = false;
        for shadow in live.iter() {
            if !shadow.sync_cpu_writes() {
                any_retired = true;
            }
        }
        live.clear();
        if live.capacity() > 4096 {
            live.shrink_to_fit();
        }
        if any_retired {
            let mut shadows = SHADOWS.lock().unwrap_or_else(|e| e.into_inner());
            shadows.retain(|weak| weak.upgrade().is_some_and(|s| !s.is_retired()));
            SHADOW_COUNT.store(shadows.len().min(u32::MAX as usize) as u32, Ordering::Release);
        }
    };
    std::thread_local! {
        static LIVE: std::cell::RefCell<Vec<Arc<ShadowPages>>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    LIVE.with(|cell| {
        if let Ok(mut live) = cell.try_borrow_mut() {
            sync_live(&mut live);
        } else {
            let mut fallback = Vec::new();
            sync_live(&mut fallback);
        }
    });
}

pub(super) fn log_shadow_probe_once() {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    crate::diagnostics::log_once(&LOGGED, || {
        "host cannot import section-backed guest pages directly; mirroring through anonymous shadow heaps"
            .into()
    });
}

fn log_shadow_once(error: &windows::core::Error) {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    crate::diagnostics::log_once(&LOGGED, || {
        format!(
            "host refused direct import of section-backed guest pages ({:?}); mirroring through anonymous shadow heaps",
            error.code()
        )
    });
}

struct ProbeSection {
    section: HANDLE,
    ptr: *mut u32,
}

impl Drop for ProbeSection {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            let _ = unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr.cast() }) };
        }
        if !self.section.is_invalid() {
            let _ = unsafe { CloseHandle(self.section) };
        }
    }
}

struct ProbeEvent(HANDLE);
impl Drop for ProbeEvent {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Probe once whether the host actually reads section-backed (`MEM_MAPPED`) guest
/// pages via `OpenExistingHeapFromAddress*`. On RADV, vkd3d-proton logs
/// `0x8007000e` internally and silently substitutes a zero-filled heap with `S_OK`.
/// Unexpected probe-setup failures return `true` to preserve default direct imports.
pub(super) fn probe_mapped_import(device: &ID3D12Device) -> bool {
    const PROBE_SIZE: usize = 65536;
    const PROBE_SENTINEL: u32 = 0x5455_5842;
    let Ok(section) = (unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            PROBE_SIZE as u32,
            None,
        )
    }) else {
        return true;
    };
    let view = unsafe { MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, PROBE_SIZE) };
    if view.Value.is_null() {
        let _ = unsafe { CloseHandle(section) };
        return true;
    }
    let mut guard = Some(ProbeSection {
        section,
        ptr: view.Value.cast(),
    });
    let ptr = guard.as_ref().unwrap().ptr;
    // SAFETY: `ptr` points to a valid 64 KiB `PAGE_READWRITE` mapped view.
    unsafe { ptr.write_volatile(PROBE_SENTINEL) };

    let result: Result<bool> = (|| {
        let heap: ID3D12Heap = if let Ok(sized) = device.cast::<ID3D12Device13>() {
            match unsafe { sized.OpenExistingHeapFromAddress1(ptr.cast(), PROBE_SIZE) } {
                Ok(h) => h,
                Err(_) => return Ok(false),
            }
        } else {
            let host: ID3D12Device3 = device.cast()?;
            match unsafe { host.OpenExistingHeapFromAddress(ptr.cast()) } {
                Ok(h) => h,
                Err(_) => return Ok(false),
            }
        };
        let src_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: 256,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER,
            ..Default::default()
        };
        let mut src: Option<ID3D12Resource> = None;
        unsafe {
            device.CreatePlacedResource(&heap, 0, &src_desc, D3D12_RESOURCE_STATE_COMMON, None, &mut src)?;
        }
        let Some(src) = src else { return Ok(true) };

        let readback_desc = D3D12_RESOURCE_DESC {
            Flags: D3D12_RESOURCE_FLAG_NONE,
            ..src_desc
        };
        let readback_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_READBACK,
            CreationNodeMask: 1,
            VisibleNodeMask: 1,
            ..Default::default()
        };
        let mut readback: Option<ID3D12Resource> = None;
        unsafe {
            device.CreateCommittedResource(
                &readback_heap,
                D3D12_HEAP_FLAG_NONE,
                &readback_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
                &mut readback,
            )?;
        }
        let Some(readback) = readback else { return Ok(true) };

        let q_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_COPY,
            ..Default::default()
        };
        let queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&q_desc)? };
        let allocator: ID3D12CommandAllocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COPY)? };
        let list: ID3D12GraphicsCommandList =
            unsafe { device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_COPY, &allocator, None)? };
        unsafe {
            list.CopyBufferRegion(&readback, 0, &src, 0, 4);
            list.Close()?;
        }
        let cmd: ID3D12CommandList = list.cast()?;
        drop(list);
        // Create fence and event before submission so every post-submit path can wait.
        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? };
        let event = ProbeEvent(unsafe { CreateEventW(None, false, false, None)? });
        unsafe {
            queue.ExecuteCommandLists(&[Some(cmd.clone())]);
        }
        let waited = unsafe {
            queue.Signal(&fence, 1).is_ok()
                && fence.SetEventOnCompletion(1, event.0).is_ok()
                && WaitForSingleObject(event.0, 5000) == WAIT_OBJECT_0
        };
        let completed = unsafe { fence.GetCompletedValue() };
        if !waited || completed < 1 {
            // The GPU copy may still be in flight; keep the probe section and COM objects alive.
            if let Some(section) = guard.take() {
                core::mem::forget(section);
            }
            core::mem::forget((cmd, allocator, queue, fence, event, readback, src, heap));
            return Ok(true);
        }
        if completed == u64::MAX {
            return Ok(true);
        }
        let range = D3D12_RANGE { Begin: 0, End: 4 };
        let empty = D3D12_RANGE { Begin: 0, End: 0 };
        let mut mapped = core::ptr::null_mut();
        unsafe { readback.Map(0, Some(&range), Some(&mut mapped))? };
        if mapped.is_null() {
            unsafe { readback.Unmap(0, Some(&empty)) };
            return Ok(true);
        }
        let word = unsafe { mapped.cast::<u32>().read() };
        unsafe { readback.Unmap(0, Some(&empty)) };
        Ok(word == PROBE_SENTINEL)
    })();
    result.unwrap_or(true)
}

pub(super) fn open_shadow_sized(
    device: &ID3D12Device13,
    base: u64,
    size: u64,
    pin: ImportPin,
    source: &'static str,
) -> Result<Arc<ImportedHeap>> {
    let len = usize::try_from(size).map_err(|_| E_INVALIDARG)?;
    let shadow = ShadowPages::new(base, len, pin.live_flag())?;
    let native = unsafe { device.OpenExistingHeapFromAddress1(shadow.ptr(), shadow.size) }.map_err(|e| {
        trace_import_failure(source, "sized-shadow", size, e.code());
        e
    })?;
    register(&shadow);
    Ok(Arc::new(ImportedHeap::shadowed(native, pin, shadow)))
}

pub(super) fn open_shadow_legacy(
    host: &ID3D12Device3,
    base: u64,
    size: u64,
    pin: ImportPin,
    source: &'static str,
) -> Result<Arc<ImportedHeap>> {
    let len = usize::try_from(size).map_err(|_| E_INVALIDARG)?;
    let shadow = ShadowPages::new(base, len, pin.live_flag())?;
    let native = unsafe { host.OpenExistingHeapFromAddress(shadow.ptr()) }.map_err(|e| {
        trace_import_failure(source, "legacy-shadow", size, e.code());
        e
    })?;
    register(&shadow);
    Ok(Arc::new(ImportedHeap::shadowed(native, pin, shadow)))
}

pub(super) fn try_open_sized(
    device: &ID3D12Device13,
    base: u64,
    size: u64,
    pin: ImportPin,
    source: &'static str,
    error: windows::core::Error,
) -> Result<Arc<ImportedHeap>> {
    if !is_userptr_refusal(&error) || !contains_mapped_pages(base, size) {
        trace_import_failure(source, "sized", size, error.code());
        return Err(error);
    }
    let heap = open_shadow_sized(device, base, size, pin, source)?;
    log_shadow_once(&error);
    Ok(heap)
}

pub(super) fn try_open_legacy(
    host: &ID3D12Device3,
    base: u64,
    size: u64,
    pin: ImportPin,
    source: &'static str,
    error: windows::core::Error,
) -> Result<Arc<ImportedHeap>> {
    if !is_userptr_refusal(&error) || !contains_mapped_pages(base, size) {
        trace_import_failure(source, "legacy", size, error.code());
        return Err(error);
    }
    let heap = open_shadow_legacy(host, base, size, pin, source)?;
    log_shadow_once(&error);
    Ok(heap)
}

#[cfg(test)]
#[path = "../../tests/device/heap_shadow.rs"]
mod tests;

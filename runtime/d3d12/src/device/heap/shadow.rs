//! Anonymous shadow copies when the host rejects section-backed host-memory imports.
use super::{ImportedHeap, trace_import_failure};
use crate::commands::page_mapping::{GPU_APERTURE, ImportPin};
use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use windows::Win32::Foundation::{CloseHandle, E_INVALIDARG, E_OUTOFMEMORY, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
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
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEM_COMMIT, MEM_MAPPED, MEM_RELEASE, MEM_RESERVE,
    MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READWRITE,
    UnmapViewOfFile, VirtualAlloc, VirtualFree, VirtualQuery,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, WaitForSingleObject};
use windows::core::{Interface, Result};

const MAX_SHADOW_BYTES: usize = 1 << 30;
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
    pin_key: usize,
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
    fn new(guest_base: u64, size: usize, pin_key: usize) -> Result<Arc<Self>> {
        if size == 0 || size > MAX_SHADOW_BYTES {
            return Err(E_INVALIDARG.into());
        }
        let mut baseline = Vec::new();
        baseline
            .try_reserve_exact(size)
            .map_err(|_| E_OUTOFMEMORY)?;
        baseline.resize(size, 0);
        // SAFETY: `None` base asks the OS for an anonymous private region of `size` bytes.
        let shadow_ptr = unsafe { VirtualAlloc(None, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        if shadow_ptr.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        let mut guard = AllocGuard(shadow_ptr.addr());
        if !read_guest_bytes(guest_base, &mut baseline) {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: `shadow_ptr` holds `size` writable bytes and `baseline` holds `size` bytes.
        unsafe { core::ptr::copy_nonoverlapping(baseline.as_ptr(), shadow_ptr.cast(), size) };
        let shadow_base = guard.0;
        guard.0 = 0;
        Ok(Arc::new(Self {
            guest_base,
            shadow_base,
            size,
            pin_key,
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

    /// Copy CPU-modified 64-byte chunks into the anonymous shadow buffer.
    /// Returns `false` once the import pin has been retired by a page remap.
    fn sync_cpu_writes(&self, scratch: &mut Vec<u8>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired {
            return false;
        }
        if !GPU_APERTURE.is_pin_live(self.pin_key) {
            state.retired = true;
            return false;
        }
        if scratch.len() < self.size && scratch.try_reserve(self.size - scratch.len()).is_err() {
            return true;
        }
        scratch.resize(self.size, 0);
        if !read_guest_bytes(self.guest_base, &mut scratch[..self.size]) {
            return true;
        }
        // SAFETY: `self.shadow_base` owns `self.size` RW bytes guarded by `self.state`.
        let shadow = unsafe { core::slice::from_raw_parts_mut(self.shadow_base as *mut u8, self.size) };
        diff_copy_chunks(&scratch[..self.size], &mut state.baseline, shadow);
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

fn read_guest_bytes(address: u64, out: &mut [u8]) -> bool {
    let process = unsafe { GetCurrentProcess() };
    let mut done = 0usize;
    while done < out.len() {
        let mut read = 0usize;
        // SAFETY: `ReadProcessMemory` validates guest readability without faulting.
        let ok = unsafe {
            ReadProcessMemory(
                process,
                address.wrapping_add(done as u64) as _,
                out[done..].as_mut_ptr().cast(),
                out.len() - done,
                Some(&mut read),
            )
        }
        .is_ok();
        if !ok || read == 0 {
            return false;
        }
        done += read;
    }
    true
}

/// Sync only 64-byte chunks where CPU memory diverged from `baseline`, leaving
/// any GPU-written chunks in `shadow` untouched.
fn diff_copy_chunks(guest: &[u8], baseline: &mut [u8], shadow: &mut [u8]) -> usize {
    let mut copied = 0usize;
    for ((g, b), s) in guest
        .chunks(CHUNK_BYTES)
        .zip(baseline.chunks_mut(CHUNK_BYTES))
        .zip(shadow.chunks_mut(CHUNK_BYTES))
    {
        if g != &*b {
            s.copy_from_slice(g);
            b.copy_from_slice(g);
            copied += g.len();
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
    let live: Vec<Arc<ShadowPages>> = {
        let mut shadows = SHADOWS.lock().unwrap_or_else(|e| e.into_inner());
        shadows.retain(|weak| weak.strong_count() != 0);
        let live: Vec<_> = shadows.iter().filter_map(Weak::upgrade).collect();
        SHADOW_COUNT.store(shadows.len().min(u32::MAX as usize) as u32, Ordering::Release);
        live
    };
    let mut any_retired = false;
    let sync_list = |scratch: &mut Vec<u8>| -> bool {
        let mut retired = false;
        for shadow in live {
            if !shadow.sync_cpu_writes(scratch) {
                retired = true;
            }
        }
        if scratch.capacity() > 4 << 20 {
            scratch.clear();
            scratch.shrink_to_fit();
        }
        retired
    };
    std::thread_local! {
        static SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    SCRATCH.with(|cell| {
        if let Ok(mut scratch) = cell.try_borrow_mut() {
            any_retired = sync_list(&mut scratch);
        } else {
            let mut local = Vec::new();
            any_retired = sync_list(&mut local);
        }
    });
    if any_retired {
        let mut shadows = SHADOWS.lock().unwrap_or_else(|e| e.into_inner());
        shadows.retain(|weak| weak.upgrade().is_some_and(|s| !s.is_retired()));
        SHADOW_COUNT.store(shadows.len().min(u32::MAX as usize) as u32, Ordering::Release);
    }
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
    let guard = ProbeSection {
        section,
        ptr: view.Value.cast(),
    };
    // SAFETY: `guard.ptr` points to a valid 64 KiB `PAGE_READWRITE` mapped view.
    unsafe { guard.ptr.write_volatile(PROBE_SENTINEL) };

    let result: Result<bool> = (|| {
        let heap: ID3D12Heap = if let Ok(sized) = device.cast::<ID3D12Device13>() {
            match unsafe { sized.OpenExistingHeapFromAddress1(guard.ptr.cast(), PROBE_SIZE) } {
                Ok(h) => h,
                Err(_) => return Ok(false),
            }
        } else {
            let host: ID3D12Device3 = device.cast()?;
            match unsafe { host.OpenExistingHeapFromAddress(guard.ptr.cast()) } {
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
        unsafe {
            queue.ExecuteCommandLists(&[Some(cmd)]);
        }
        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? };
        let event = ProbeEvent(unsafe { CreateEventW(None, false, false, None)? });
        unsafe {
            queue.Signal(&fence, 1)?;
            fence.SetEventOnCompletion(1, event.0)?;
            if WaitForSingleObject(event.0, 5000) != WAIT_OBJECT_0 {
                return Ok(true);
            }
            let completed = fence.GetCompletedValue();
            if completed < 1 || completed == u64::MAX {
                return Ok(true);
            }
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
    let shadow = ShadowPages::new(base, len, pin.key())?;
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
    let shadow = ShadowPages::new(base, len, pin.key())?;
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

//! Host commit admission for the layer's own upload growth.
//!
//! A title allocates against the machine's real commit limit. When this layer
//! grows upload storage while the host is short of commit, the title's own
//! allocations are what fail, and the run black-screens with a residency
//! warning. Ported measurement: `codec-d/cool_fox`'s `residency.rs`, via
//! `cesarzxk`'s CGBJ fork (BR-41). Their floors, their sample cadence.
//!
//! `TUXBOX_COMMIT_ADMISSION=1` enables the refusal. The sample and the
//! transition log run either way, so a boot reports what the host looked like
//! before anything depends on the answer.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const MIB: u64 = 1 << 20;

/// Physical free memory below which the layer stops growing upload storage.
const PRESSURE_PHYSICAL: u64 = 5 * (1 << 30);
/// Commit below which the layer stops growing upload storage.
const PRESSURE_COMMIT: u64 = 6 * (1 << 30);
/// Commit at which growth is allowed again. The gap stops a floor from flapping.
const CLEAR_COMMIT: u64 = 4 * (1 << 30);
const CLEAR_PHYSICAL: u64 = 3 * (1 << 30);
/// Below this, a refused allocation is an emergency rather than a delay.
const CRITICAL_COMMIT: u64 = 1536 * MIB;

static AVAILABLE_COMMIT: AtomicU64 = AtomicU64::new(u64::MAX);
static AVAILABLE_PHYSICAL: AtomicU64 = AtomicU64::new(u64::MAX);
static PRESSURED: AtomicBool = AtomicBool::new(false);
static NEXT_SAMPLE: AtomicU64 = AtomicU64::new(0);

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("TUXBOX_COMMIT_ADMISSION").is_some_and(|value| value == "1")
    })
}

fn memory_status() -> Option<MEMORYSTATUSEX> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: the length field describes this writable structure.
    unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok().then_some(status)
}

/// Millisecond clock for the sample cadence. `GetTickCount64` is coarse enough.
fn now_ms() -> u64 {
    // SAFETY: the call takes no arguments and cannot fault.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// Reserve the layer keeps before it grows upload storage by `bytes`.
pub(crate) fn reserve_for(bytes: u64) -> u64 {
    (512 * MIB).saturating_add(bytes)
}

/// Whether a host with `commit` bytes available may grow by `bytes`.
pub(crate) fn allow(commit: u64, bytes: u64) -> bool {
    commit >= reserve_for(bytes)
}

/// Whether a sample reads as pressure. Hysteresis: entering needs less headroom
/// than leaving, so a host hovering at the floor does not flap.
pub(crate) fn pressure(was: bool, physical: u64, commit: u64) -> bool {
    if was {
        physical < PRESSURE_PHYSICAL || commit < PRESSURE_COMMIT
    } else {
        physical <= CLEAR_PHYSICAL || commit <= CLEAR_COMMIT
    }
}

fn sample() -> (u64, u64) {
    let now = now_ms();
    let next = NEXT_SAMPLE.load(Ordering::Relaxed);
    if now >= next
        && NEXT_SAMPLE
            .compare_exchange(next, now.saturating_add(1000), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        && let Some(status) = memory_status()
    {
        AVAILABLE_PHYSICAL.store(status.ullAvailPhys, Ordering::Relaxed);
        AVAILABLE_COMMIT.store(status.ullAvailPageFile, Ordering::Relaxed);
    }
    (
        AVAILABLE_PHYSICAL.load(Ordering::Relaxed),
        AVAILABLE_COMMIT.load(Ordering::Relaxed),
    )
}

/// Admit (or refuse) growing upload storage by `bytes`.
pub(crate) fn admit(bytes: u64) -> bool {
    let (physical, commit) = sample();
    if physical == u64::MAX || commit == u64::MAX {
        return true;
    }
    let was = PRESSURED.load(Ordering::Relaxed);
    let now = pressure(was, physical, commit);
    if now != was {
        PRESSURED.store(now, Ordering::Relaxed);
        static SAMPLE_LINE: std::sync::Once = std::sync::Once::new();
        crate::diagnostics::log_once(&SAMPLE_LINE, || {
            format!(
                "host memory: physical_free_mib={} available_commit_mib={} growth_mib={} pressured={now} admission={}",
                physical >> 20,
                commit >> 20,
                bytes >> 20,
                if enabled() { "on" } else { "off" },
            )
        });
    }
    if !enabled() {
        return true;
    }
    allow(commit, bytes)
}

/// True when commit is so low that a refusal is an emergency, not a delay.
pub(crate) fn critical() -> bool {
    sample().1 < CRITICAL_COMMIT
}

#[cfg(test)]
mod tests {
    use super::{allow, pressure, reserve_for};

    #[test]
    fn a_small_growth_keeps_half_a_gibibyte_back() {
        assert_eq!(reserve_for(0), 512 << 20);
    }

    #[test]
    fn the_reserve_rises_with_the_request() {
        let big = 64 << 20;
        assert_eq!(reserve_for(big), (512 << 20) + big);
    }

    #[test]
    fn a_host_below_the_reserve_is_refused() {
        assert!(!allow(500 << 20, 0));
    }

    #[test]
    fn a_host_with_headroom_is_admitted() {
        assert!(allow(2 << 30, 4 << 20));
    }

    #[test]
    fn pressure_needs_more_headroom_to_enter_than_to_leave() {
        // Entering: below 5 GiB physical or 6 GiB commit.
        assert!(pressure(false, 4 << 30, 100 << 30));
        assert!(pressure(false, 100 << 30, 5 << 30));
        // Leaving: needs more than 3 GiB physical and 4 GiB commit.
        assert!(!pressure(true, 4 << 30, 100 << 30));
    }
}

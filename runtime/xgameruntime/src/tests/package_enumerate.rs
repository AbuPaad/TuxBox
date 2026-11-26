use super::*;

/// The title passes a null callback when it wants the count or does not care
/// about entries. An empty enumeration is success, not a pointer fault.
#[test]
fn a_null_callback_is_still_success() {
    let result = package_enumerate(&crate::THREAD_OBJECT, 1, 0, core::ptr::null_mut(), core::ptr::null_mut());
    assert_eq!(result, crate::OK);
}

#[test]
fn a_callback_is_never_invoked_because_no_package_is_installed() {
    static CALLS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    unsafe extern "system" fn callback(_context: *mut core::ffi::c_void, _details: *const core::ffi::c_void) -> i32 {
        CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        0
    }
    let result = package_enumerate(
        &crate::THREAD_OBJECT,
        1,
        0,
        core::ptr::null_mut(),
        callback as *mut core::ffi::c_void,
    );
    assert_eq!(result, crate::OK);
    assert_eq!(CALLS.load(core::sync::atomic::Ordering::Relaxed), 0);
}

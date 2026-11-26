use super::*;

#[test]
fn the_record_is_the_size_the_header_declares() {
    // XVersion twice, then two 64-byte descriptions.
    assert_eq!(core::mem::size_of::<AnalyticsInfo>(), 144);
    assert_eq!(core::mem::size_of::<Version>(), 8);
}

#[test]
fn the_versions_describe_a_retail_series_s() {
    let info = analytics_info();
    assert_eq!(info.os_version, TITLE_OS);
    assert_eq!(info.hosting_os_version, TITLE_OS);
    assert_eq!(TITLE_OS.major, 10);
    assert_eq!(TITLE_OS.build, 25398);
}

#[test]
fn the_descriptions_are_nul_terminated_inside_their_fields() {
    let info = analytics_info();
    assert_eq!(&info.family[..5], b"Xbox\0");
    assert_eq!(&info.form[..14], b"Xbox Series S\0");
    assert!(info.family[5..].iter().all(|&b| b == 0));
    assert!(info.form[14..].iter().all(|&b| b == 0));
}

#[test]
fn a_longer_description_is_truncated_and_still_terminated() {
    let truncated = text::<8>("Xbox Series S");
    assert_eq!(&truncated, b"Xbox Se\0");
}

#[test]
fn the_first_plausible_argument_wins() {
    // The shape ProjectScarlett passes: a buffer first, then a small tag.
    assert_eq!(analytics_buffer(0x6ea0_f600, 1), Some(0x6ea0_f600));
}

#[test]
fn a_leading_size_falls_through_to_the_second_argument() {
    assert_eq!(analytics_buffer(144, 0x6f2f_6a90), Some(0x6f2f_6a90));
}

#[test]
fn neither_argument_plausible_is_no_buffer() {
    assert_eq!(analytics_buffer(144, 1), None);
    assert_eq!(analytics_buffer(0, 0), None);
}

#[test]
fn a_missing_buffer_answers_null() {
    // The title copies from what this returns, so it must never invent a value.
    let result = unsafe { system_analytics_info(&crate::THREAD_OBJECT, 0, 0) };
    assert!(result.is_null());
}

#[test]
fn the_write_fills_the_buffer_and_returns_it() {
    let mut info = core::mem::MaybeUninit::<AnalyticsInfo>::zeroed();
    let address = info.as_mut_ptr() as usize;
    let returned = unsafe { system_analytics_info(&crate::THREAD_OBJECT, address, 1) };
    // The stub copies 144 bytes from the pointer, so it must be the buffer.
    assert_eq!(returned as usize, address);
    let info = unsafe { info.assume_init() };
    assert_eq!(info.os_version, TITLE_OS);
    assert_eq!(&info.family[..5], b"Xbox\0");
    assert_eq!(&info.form[..14], b"Xbox Series S\0");
}

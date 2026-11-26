use super::*;

#[test]
fn signed_normalized_codes_use_the_d3d_divisor() {
    // BUFFER_FORMAT_16_16_SNORM. D3D divides by 32767, so the one code below
    // -1 clamps to -1.0 instead of reaching -1.00003.
    assert_eq!(snorm16(32767), 1.0);
    assert_eq!(snorm16(0), 0.0);
    assert_eq!(snorm16(-32768), -1.0);
    // The midpoint is a little above 0.5 because the range is asymmetric:
    // 16384 / 32767, not 16384 / 32768.
    assert!((snorm16(16384) - 0.500_015_26).abs() < 1e-7, "{}", snorm16(16384));
    assert!((snorm16(-1) - (-1.0 / 32767.0)).abs() < 1e-12);
}

#[test]
fn each_channel_reads_its_own_half_of_the_dword() {
    let first = snorm16_hlsl("packed", 0);
    let second = snorm16_hlsl("packed", 1);
    // Channel 0 is the low half: shift it up, then sign-extend it back down.
    assert!(first.contains("(packed << 16u) >> 16"), "{first}");
    assert!(!first.contains("<< 0u"), "{first}");
    // Channel 1 is already in place, so only the sign extension applies.
    assert!(second.contains("(packed << 0u) >> 16"), "{second}");
    assert!(!second.contains("<< 16u"), "{second}");
}

#[test]
fn every_channel_divides_and_clamps_the_same_way() {
    for channel in 0..2 {
        let hlsl = snorm16_hlsl("packed", channel);
        assert!(hlsl.contains("/ 32767.0f"), "{hlsl}");
        assert!(hlsl.contains("max(-1.0f,"), "{hlsl}");
        // The result must come back as a bit pattern, because the destination
        // register is read as a float.
        assert!(hlsl.starts_with("asuint("), "{hlsl}");
    }
}

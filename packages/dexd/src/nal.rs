//! The asset check: reads the leading NAL units of a raw Annex-B HEVC stream
//! and refuses a video that cannot restart at byte 0 as an ordinary keyframe.
//!
//! The first slice must be an IDR (nal_unit_type 19 or 20). A CRA (21) starts
//! an open GOP, so whether it loops cleanly depends on the content; it is
//! refused by name, and every other non-IDR first slice is refused with its
//! type number. See docs/design/sidecar.md#the-asset-check.

/// HEVC nal_unit_type values (H.265 Table 7-1) this check names.
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;
pub const NAL_IDR_W_RADL: u8 = 19;
pub const NAL_IDR_N_LP: u8 = 20;

/// Validate the leading NAL units of a raw Annex-B HEVC stream.
///
/// Passes if VPS, SPS and PPS have all appeared before the first VCL NAL
/// (types 0-31) and that first VCL NAL is an IDR (19 or 20). Everything after
/// the first VCL NAL is out of scope, so a truncated copy passes here; the
/// sidecar checksum covers the bytes of the whole video.
///
/// See docs/design/sidecar.md#the-asset-check.
pub fn validate_leading_nals(data: &[u8]) -> Result<(), String> {
    let mut vps = false;
    let mut sps = false;
    let mut pps = false;
    let mut found_any = false;
    let mut iter = StartCodeIter { data, i: 0 };
    while let Some((b0, _b1)) = iter.next_nal_header() {
        found_any = true;
        if b0 & 0x80 != 0 {
            return Err("corrupt NAL header (forbidden_zero_bit set)".into());
        }
        let nal_type = (b0 >> 1) & 0x3f;
        match nal_type {
            NAL_VPS => vps = true,
            NAL_SPS => sps = true,
            NAL_PPS => pps = true,
            0..=31 => {
                // First VCL NAL: the check decides here.
                let missing: Vec<&str> = [(!vps, "VPS"), (!sps, "SPS"), (!pps, "PPS")]
                    .iter()
                    .filter(|(m, _)| *m)
                    .map(|(_, n)| *n)
                    .collect();
                if !missing.is_empty() {
                    return Err(format!(
                        "first slice appears before parameter sets ({} missing); prepare \
                         the video again with a closed-GOP encode",
                        missing.join("/")
                    ));
                }
                return match nal_type {
                    NAL_IDR_W_RADL | NAL_IDR_N_LP => Ok(()),
                    21 => Err(
                        "leading keyframe is CRA (open GOP), not IDR; the picture would \
                         break at the loop point — prepare the video again with a \
                         closed-GOP encode (IDR at frame 0)"
                            .into(),
                    ),
                    t => Err(format!(
                        "first slice NAL is type {t}, not an IDR (19/20); the stream does \
                         not start on a clean keyframe — prepare the video again with \
                         a closed-GOP encode"
                    )),
                };
            }
            _ => {} // other non-VCL units before the IDR are fine
        }
    }
    if !found_any {
        return Err(
            "no Annex-B start code found; this is not a raw HEVC elementary stream (MP4? \
             use: ffmpeg -i in.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc out.265)"
                .into(),
        );
    }
    Err("parameter sets but no slice found in the asset".into())
}

/// Yields the two NAL header bytes after each `00 00 01` start code; the
/// four-byte start-code form contains that pattern, so one search finds both.
///
/// See docs/design/sidecar.md#the-asset-check.
struct StartCodeIter<'a> {
    data: &'a [u8],
    i: usize,
}

impl StartCodeIter<'_> {
    fn next_nal_header(&mut self) -> Option<(u8, u8)> {
        let d = self.data;
        let mut i = self.i;
        while i + 2 < d.len() {
            if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
                let h = i + 3;
                self.i = h + 1; // keep searching after this start code
                if h + 1 < d.len() {
                    return Some((d[h], d[h + 1]));
                }
                return None; // start code at the end of the data, no room for a header
            }
            i += 1;
        }
        self.i = i;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One NAL: 4-byte start code + 2-byte header (layer 0, tid+1 = 1).
    fn nal(nal_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, nal_type << 1, 0x01];
        v.extend_from_slice(payload);
        v
    }

    fn stream(types: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        for &t in types {
            v.extend(nal(t, &[0x2a; 8]));
        }
        v
    }

    #[test]
    fn parameter_sets_then_an_idr_pass() {
        // IDR_W_RADL, then IDR_N_LP
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 19])).is_ok());
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 20])).is_ok());
        // non-VCL units before and among the parameter sets are fine (types 35 and 39)
        assert!(validate_leading_nals(&stream(&[35, 32, 39, 33, 34, 19])).is_ok());
        // slices after the IDR are out of scope
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 19, 1, 0, 1])).is_ok());
    }

    #[test]
    fn three_byte_start_codes_pass_too() {
        let mut v = Vec::new();
        for t in [32u8, 33, 34, 19] {
            v.extend([0, 0, 1, t << 1, 0x01]);
            v.extend([0x2a; 8]);
        }
        assert!(validate_leading_nals(&v).is_ok());
    }

    #[test]
    fn missing_parameter_sets_are_refused_and_named() {
        let e = validate_leading_nals(&stream(&[32, 33, 19])).unwrap_err();
        assert!(e.contains("PPS"), "{e}");
        let e = validate_leading_nals(&stream(&[34, 19])).unwrap_err();
        assert!(e.contains("VPS") && e.contains("SPS"), "{e}");
    }

    #[test]
    fn a_first_slice_that_is_not_an_idr_is_refused() {
        // TRAIL_R (type 1) as the first slice: not a keyframe.
        let e = validate_leading_nals(&stream(&[32, 33, 34, 1])).unwrap_err();
        assert!(e.contains("not an IDR"), "{e}");
    }

    #[test]
    fn a_cra_open_gop_is_refused_and_named() {
        let e = validate_leading_nals(&stream(&[32, 33, 34, 21])).unwrap_err();
        assert!(e.contains("CRA"), "{e}");
    }

    #[test]
    fn parameter_sets_after_the_slice_do_not_count() {
        let e = validate_leading_nals(&stream(&[32, 33, 19, 34])).unwrap_err();
        assert!(e.contains("PPS"), "{e}");
    }

    #[test]
    fn empty_and_malformed_input_is_refused() {
        assert!(validate_leading_nals(&[]).is_err());
        // no Annex-B start code at all
        let e = validate_leading_nals(&[0x47; 4096]).unwrap_err();
        assert!(e.contains("start code"), "{e}");
        // parameter sets only, no slice ever
        assert!(validate_leading_nals(&stream(&[32, 33, 34])).is_err());
        // forbidden_zero_bit set on the first NAL header
        let mut v = vec![0, 0, 0, 1, 0x80 | (32 << 1), 0x01];
        v.extend([0x2a; 8]);
        assert!(validate_leading_nals(&v).is_err());
    }
}

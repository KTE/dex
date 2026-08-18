//! F4 — the asset validation gate: refuse an asset whose leading NALs cannot
//! support the gaplessness premise.
//!
//! The endless-stream design only wraps seamlessly because byte 0 begins a
//! closed GOP: parameter sets (VPS/SPS/PPS) then an IDR, so re-entering at
//! byte 0 mid-stream is an ordinary keyframe, not a seek. An asset that
//! starts with anything else — an open-GOP CRA, a trailing slice, no
//! parameter sets — would "play" and then glitch at EVERY wrap (~29k visible
//! artefacts/day for a 3 s loop), silently. The hash (F3) proves the bytes
//! are the ingested bytes; this gate proves the ingested bytes have the
//! required SHAPE. Truncation is F3's job: a truncated copy has intact
//! leading NALs and passes this gate by design.
//!
//! Why IDR only (19/20), not any IRAP (16-23): CRA (21) admits RASL leading
//! pictures whose wrap-join correctness depends on the content; BLA (16-18)
//! never comes from a sane ingest; 22/23 are reserved. The premise stated
//! everywhere in this crate is "IDR at frame 0" — so that is what the gate
//! enforces. Relax knowingly if an asset ever justifies it.

/// HEVC nal_unit_type values (ITU-T H.265 Table 7-1) this gate names.
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;
pub const NAL_IDR_W_RADL: u8 = 19;
pub const NAL_IDR_N_LP: u8 = 20;

/// Validate the leading NAL units of a raw Annex-B HEVC stream.
///
/// Passes iff, before the first VCL NAL (type 0-31), all of VPS/SPS/PPS have
/// appeared, and that first VCL NAL is an IDR (19 or 20). Everything after
/// the first VCL NAL is out of scope — the F3 hash covers byte-level
/// integrity of the whole asset.
///
/// Scanning is a plain 00 00 01 search (3- and 4-byte start codes both
/// resolve to it): encoders insert emulation-prevention bytes precisely so
/// that pattern never occurs inside a NAL payload, so the search cannot
/// false-positive on a well-formed stream.
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
                // First VCL NAL: the gate's decision point.
                let missing: Vec<&str> = [(!vps, "VPS"), (!sps, "SPS"), (!pps, "PPS")]
                    .iter()
                    .filter(|(m, _)| *m)
                    .map(|(_, n)| *n)
                    .collect();
                if !missing.is_empty() {
                    return Err(format!(
                        "first slice appears before parameter sets ({} missing); not a \
                         valid loop asset — prepare the video again with a closed-GOP encode",
                        missing.join("/")
                    ));
                }
                return match nal_type {
                    NAL_IDR_W_RADL | NAL_IDR_N_LP => Ok(()),
                    21 => Err(
                        "leading keyframe is CRA (open GOP), not IDR; the wrap would \
                         splice mid-GOP — prepare the video again with a closed-GOP \
                         encode (IDR at \
                         frame 0)"
                            .into(),
                    ),
                    t => Err(format!(
                        "first slice NAL is type {t}, not an IDR (19/20); the stream does \
                         not start on a clean keyframe — prepare the video again with \
                         a closed-GOP encode"
                    )),
                };
            }
            _ => {} // other non-VCL (AUD, SEI, ...): fine before the IDR
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

/// Finds each 00 00 01 start code (the 4-byte form contains it) and yields
/// the two NAL header bytes that follow.
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
                return None; // start code at EOF, no room for a header
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
    fn valid_closed_gop_asset_passes() {
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 19])).is_ok()); // IDR_W_RADL
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 20])).is_ok()); // IDR_N_LP
                                                                            // non-VCL noise before/among parameter sets is fine (AUD=35, SEI=39)
        assert!(validate_leading_nals(&stream(&[35, 32, 39, 33, 34, 19])).is_ok());
        // trailing slices after the IDR are out of scope for the gate
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
    fn non_idr_first_slice_is_refused() {
        // TRAIL_R (1) first: a copy that lost its head, or a cut mid-GOP.
        // Would glitch at EVERY wrap.
        let e = validate_leading_nals(&stream(&[32, 33, 34, 1])).unwrap_err();
        assert!(e.contains("not an IDR"), "{e}");
    }

    #[test]
    fn cra_open_gop_is_refused_by_name() {
        let e = validate_leading_nals(&stream(&[32, 33, 34, 21])).unwrap_err();
        assert!(e.contains("CRA"), "{e}");
    }

    #[test]
    fn parameter_sets_after_the_slice_do_not_count() {
        let e = validate_leading_nals(&stream(&[32, 33, 19, 34])).unwrap_err();
        assert!(e.contains("PPS"), "{e}");
    }

    #[test]
    fn garbage_and_degenerate_inputs_are_refused() {
        assert!(validate_leading_nals(&[]).is_err());
        let e = validate_leading_nals(&[0x47; 4096]).unwrap_err();
        assert!(e.contains("start code"), "{e}"); // no Annex-B start code at all
                                                  // parameter sets only, no slice ever
        assert!(validate_leading_nals(&stream(&[32, 33, 34])).is_err());
        // forbidden_zero_bit set on the first NAL header
        let mut v = vec![0, 0, 0, 1, 0x80 | (32 << 1), 0x01];
        v.extend([0x2a; 8]);
        assert!(validate_leading_nals(&v).is_err());
    }
}

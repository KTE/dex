//! T0 — the wrap arithmetic of the endless stream, extracted pure so it is
//! testable without libmpv. `read_fn` in main.rs is a thin unsafe shell over
//! `next_chunk`; the properties asserted here (never a zero-byte answer, the
//! concatenation property) ARE the program.

/// One read request's answer: copy `n` bytes starting at payload offset
/// `start`; the reader's position afterwards is `next_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// Offset into the payload to copy from. Always < payload length.
    pub start: usize,
    /// Bytes to copy. Never 0.
    pub n: usize,
    /// Reader position after the copy. Always < payload length: the wrap
    /// happens eagerly here, never lazily on the next call, so the invariant
    /// holds between calls.
    pub next_pos: usize,
}

/// Decide the next chunk of the endless loop.
///
/// `len` is the payload length, `pos` the reader position (any value is
/// tolerated; positions >= len wrap to 0 first), `want` the requested byte
/// count.
///
/// Returns `None` exactly when no bytes can be produced without lying: a
/// zero-length request (`want == 0`), or the impossible-after-startup empty
/// payload (`len == 0`). The caller MUST turn `None` into an mpv ERROR
/// return — never 0. To mpv, 0 means final EOF (stream_cb.h), the one event
/// this program exists to prevent, and "0 bytes requested" must never share a
/// return value with "the stream has ended".
///
/// A short read is legal (stream_cb.h), so the wrap is never stitched across
/// one call: the tail is returned now, the head on the next call.
pub fn next_chunk(len: usize, pos: usize, want: usize) -> Option<Chunk> {
    if len == 0 || want == 0 {
        return None;
    }
    let start = if pos >= len { 0 } else { pos };
    let avail = len - start; // >= 1, because start < len
    let n = want.min(avail); // >= 1, because want >= 1 and avail >= 1
    let end = start + n; // <= len
    let next_pos = if end == len { 0 } else { end };
    Some(Chunk { start, n, next_pos })
}

/// Clamp mpv's u64 request size to usize without ever turning a nonzero
/// request into 0. On a 32-bit target `as usize` truncates: an `nbytes` that
/// is an exact multiple of 2^32 would become a 0-byte request and therefore a
/// spurious final EOF. Saturating can only shrink the request, and short
/// reads are always legal.
pub fn clamp_want(nbytes: u64) -> usize {
    usize::try_from(nbytes).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1: never returns n == 0 for any want >= 1 (a 0 return = final EOF to
    // mpv, the exact event the design exists to prevent).
    #[test]
    fn never_zero_bytes_for_nonzero_want() {
        for len in [1usize, 2, 3, 7, 64, 1000] {
            for pos in [0usize, 1, len / 2, len.saturating_sub(1), len, len + 5] {
                for want in [1usize, 2, len, len + 1, 10 * len, usize::MAX] {
                    let c = next_chunk(len, pos, want)
                        .expect("want >= 1 on a non-empty payload must produce a chunk");
                    assert!(c.n >= 1, "n == 0 for len={len} pos={pos} want={want}");
                }
            }
        }
    }

    // T1: want == 0 (and the impossible len == 0) are explicit, distinct
    // outcomes — never conflated with a zero-byte "success" that mpv would
    // read as EOF.
    #[test]
    fn zero_want_and_empty_payload_are_none_not_zero_chunks() {
        assert_eq!(next_chunk(10, 0, 0), None);
        assert_eq!(next_chunk(10, 9, 0), None);
        assert_eq!(next_chunk(1, 0, 0), None);
        assert_eq!(next_chunk(10, 10, 0), None); // even at the wrap point
        assert_eq!(next_chunk(0, 0, 4096), None); // empty payload: error, not EOF
    }

    // T1: wrap at the exact payload boundary.
    #[test]
    fn wraps_at_exact_boundary() {
        // pos at end-of-payload (legacy lazy-caller state): wraps to 0 first.
        let c = next_chunk(10, 10, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 4, 4));
        // tail shorter than want: short read of the tail, next_pos wraps to 0.
        let c = next_chunk(10, 8, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (8, 2, 0));
        // read ending exactly at len: next_pos is 0, not len (eager wrap).
        let c = next_chunk(10, 6, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (6, 4, 0));
    }

    // T1: payload smaller than the request.
    #[test]
    fn payload_smaller_than_want() {
        // whole payload in one request: short read of everything, wrap to 0.
        let c = next_chunk(3, 0, 4096).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 3, 0));
        // single-byte payload: every read returns that byte, forever.
        for _ in 0..5 {
            let c = next_chunk(1, 0, 4096).unwrap();
            assert_eq!((c.start, c.n, c.next_pos), (0, 1, 0));
        }
    }

    // T1: the position invariant read_fn's SAFETY comment relies on.
    #[test]
    fn next_pos_always_less_than_len() {
        for len in [1usize, 2, 3, 5, 64, 4096] {
            for pos in 0..=len + 2 {
                for want in [1usize, 2, 3, len, len + 1, 3 * len] {
                    let c = next_chunk(len, pos, want).unwrap();
                    assert!(c.next_pos < len, "next_pos={} len={len}", c.next_pos);
                    assert!(c.start < len);
                    assert!(c.start + c.n <= len);
                }
            }
        }
    }

    // T1: THE property — driving next_chunk repeatedly reproduces the payload
    // repeated endlessly, byte for byte, for arbitrary request sizes. This is
    // the only property that matters: the stream really is the loop.
    #[test]
    fn concatenation_reproduces_the_endless_loop() {
        let payload: Vec<u8> = (0u8..=250).cycle().take(997).collect(); // prime length
                                                                        // deterministic pseudo-random request sizes (LCG, no dependencies)
        let mut rng: u64 = 0x853c49e6748fea9b;
        let mut random_sizes = Vec::new();
        for _ in 0..2000 {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            random_sizes.push(((rng >> 33) % 300 + 1) as usize); // 1..=300
        }
        let schedules: Vec<Vec<usize>> = vec![
            vec![1; 3000], // one byte at a time
            vec![997; 8],  // exactly the payload length
            vec![996; 8],  // one short of the payload
            vec![998; 8],  // one past the payload
            vec![4096; 8], // far larger than the payload
            random_sizes,  // pseudo-random schedule
        ];
        for schedule in schedules {
            let mut pos = 0usize;
            let mut out = Vec::new();
            for want in &schedule {
                let c = next_chunk(payload.len(), pos, *want).unwrap();
                out.extend_from_slice(&payload[c.start..c.start + c.n]);
                pos = c.next_pos;
            }
            let expected: Vec<u8> = payload.iter().copied().cycle().take(out.len()).collect();
            assert_eq!(out, expected, "stream diverged from the endless loop");
        }
    }

    // T1: the u64 -> usize clamp saturates rather than truncating to 0.
    // Honest limitation: on a 64-bit host try_from always succeeds, so the
    // truncation branch only genuinely executes on a 32-bit target (the Pi 4
    // runs aarch64). This pins the contract against someone reintroducing
    // `as usize`; a 32-bit CI target would then catch it.
    #[test]
    fn clamp_want_never_zero_for_nonzero_input() {
        assert_eq!(clamp_want(0), 0);
        assert_eq!(clamp_want(1), 1);
        assert_eq!(clamp_want(4096), 4096);
        assert!(clamp_want(1u64 << 32) != 0, "2^32 must not clamp to 0");
        assert!(clamp_want(u64::MAX) != 0);
        assert_eq!(clamp_want(u64::MAX), usize::MAX);
    }
}

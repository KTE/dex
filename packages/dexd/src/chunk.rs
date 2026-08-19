//! Loop-position arithmetic for the endless stream, kept free of libmpv so it
//! can be tested on its own. The read callback in main.rs is a thin unsafe
//! shell over `next_chunk`; the tests here enforce the rules that callback
//! depends on: never a zero-byte answer, the bounds the unsafe copy relies on,
//! and byte-for-byte reproduction of the endlessly repeated payload.
//!
//! See docs/design/endless-stream.md#loop-position-arithmetic.

/// One read request's answer: copy `n` bytes starting at payload offset
/// `start`; the reader's position afterwards is `next_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// Offset into the payload to copy from. Always below the payload length.
    pub start: usize,
    /// Bytes to copy. Never 0.
    pub n: usize,
    /// Reader position after the copy. Always below the payload length: the
    /// position returns to 0 as soon as a copy reaches the end of the payload,
    /// so the bounds the unsafe copy relies on hold between calls.
    pub next_pos: usize,
}

/// Decide the next chunk of the endless loop.
///
/// `len` is the payload length, `pos` the reader position (any value is
/// tolerated; a position at or past `len` starts again at 0), `want` the
/// requested byte count.
///
/// Returns `None` exactly when no bytes can be produced: a zero-length
/// request, or an empty payload. The caller turns that into an mpv error
/// rather than 0, which keeps the case visible in dexd's own diagnostics.
///
/// A short read is legal (`stream_cb.h`), so one call never returns bytes from
/// both the end and the start of the payload: the tail comes now, the head on
/// the next call.
///
/// See docs/design/endless-stream.md#loop-position-arithmetic.
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

/// Convert mpv's `u64` request size to `usize`, saturating at `usize::MAX`.
///
/// Saturating can only shrink the request, and a short read is legal;
/// `as usize` would truncate a multiple of 2^32 to a 0-byte request on a
/// 32-bit target.
///
/// See docs/design/endless-stream.md#loop-position-arithmetic.
pub fn clamp_want(nbytes: u64) -> usize {
    usize::try_from(nbytes).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 0-byte answer is final end-of-file to mpv, the event the endless stream
    // exists to prevent.
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

    // A zero-length request and the empty payload are separate outcomes, kept
    // apart from a zero-byte success that mpv would read as end-of-file.
    #[test]
    fn zero_want_and_empty_payload_produce_no_chunk() {
        assert_eq!(next_chunk(10, 0, 0), None);
        assert_eq!(next_chunk(10, 9, 0), None);
        assert_eq!(next_chunk(1, 0, 0), None);
        assert_eq!(next_chunk(10, 10, 0), None); // even at the end of the payload
        assert_eq!(next_chunk(0, 0, 4096), None); // empty payload: error, not end-of-file
    }

    // The copy and the next position when a request reaches the end of the payload.
    #[test]
    fn position_returns_to_zero_at_the_payload_boundary() {
        // position already at the end of the payload: the copy starts at 0.
        let c = next_chunk(10, 10, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 4, 4));
        // tail shorter than want: short read of the tail, next_pos returns to 0.
        let c = next_chunk(10, 8, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (8, 2, 0));
        // a copy ending at len: next_pos is 0, so the bounds the unsafe copy
        // relies on hold between calls.
        let c = next_chunk(10, 6, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (6, 4, 0));
    }

    // A request larger than the whole payload.
    #[test]
    fn payload_smaller_than_want() {
        // whole payload in one request: short read of everything, and the position
        // returns to 0.
        let c = next_chunk(3, 0, 4096).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 3, 0));
        // single-byte payload: every read returns that byte, forever.
        for _ in 0..5 {
            let c = next_chunk(1, 0, 4096).unwrap();
            assert_eq!((c.start, c.n, c.next_pos), (0, 1, 0));
        }
    }

    // The position invariant the read callback's SAFETY comment relies on.
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

    // Driving next_chunk repeatedly reproduces the endlessly repeated payload,
    // byte for byte, for arbitrary request sizes.
    #[test]
    fn concatenation_reproduces_the_endless_loop() {
        let payload: Vec<u8> = (0u8..=250).cycle().take(997).collect(); // prime length

        // Deterministic pseudo-random request sizes, from a linear congruential
        // generator so that no dependency is needed.
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
            vec![997; 8],  // the payload length itself
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

    // The clamp saturates. `try_from` always succeeds on the 64-bit target, so
    // this test holds the contract against a reintroduced `as usize`.
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

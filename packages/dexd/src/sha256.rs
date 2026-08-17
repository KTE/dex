//! F3 prerequisite — one-shot SHA-256 (FIPS 180-4) for the asset<->sidecar
//! binding, over the `sha2` crate.
//!
//! This module was hand-rolled (185 lines, FIPS 180-4 from scratch) under the
//! crate's former zero-dependency rule. That rule is withdrawn — SPEC §5c —
//! so the implementation is now RustCrypto's and this file is a thin wrapper.
//!
//! **The wrapper is deliberate: the module keeps its API so the vectors below
//! keep testing the thing the player actually calls.** They now pin `sha2`
//! rather than our own compression function, which is the point — the tests
//! were always a statement about `sha256_hex`'s output, never about who
//! computed it.
//!
//! Before the swap, the hand-rolled implementation was confirmed to agree with
//! coreutils `sha256sum` on the real bench assets (loop4k.265, loop.265). That
//! check is why existing sidecars remain valid across this change: had it
//! disagreed, every sidecar in the field would have encoded a wrong digest and
//! this "refactor" would have turned the F3 startup gate into a fleet-wide
//! boot loop. A hash change is a data-format change.

use sha2::{Digest, Sha256};

/// SHA-256 of `data`, one shot.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(data));
    out
}

/// SHA-256 of `data` as 64 lowercase hex characters — the sidecar format.
pub fn sha256_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let d = sha256(data);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_times(n: usize) -> Vec<u8> {
        vec![b'a'; n]
    }

    #[test]
    fn nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    // The NIST long vector: one million 'a'. Exercises many blocks and the
    // rem == 0 padding path on a large input. Milliseconds even in debug.
    #[test]
    fn nist_million_a() {
        assert_eq!(
            sha256_hex(&a_times(1_000_000)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    // Padding boundaries: 55 is the last 1-padding-block length, 56 the first
    // 2-block one; 63/64/65 straddle the block size; 112 covers a mid-size
    // rem. Reference digests generated with `shasum -a 256` on 2026-08-15.
    #[test]
    fn padding_boundaries() {
        for (n, want) in [
            (
                55,
                "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
            ),
            (
                56,
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
            ),
            (
                63,
                "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
            ),
            (
                64,
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
            (
                65,
                "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
            ),
            (
                112,
                "f54353008a2553262ecdc4a34749563ba0950e8b0fc8652780b0a614b99683c1",
            ),
        ] {
            assert_eq!(sha256_hex(&a_times(n)), want, "length {n}");
        }
    }
}

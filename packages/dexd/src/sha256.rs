//! SHA-256 for the asset checksum: a thin wrapper over the `sha2` crate.
//!
//! The module keeps its own API so the test vectors below check the digest
//! the player computes, whatever computes it. The digest is part of the
//! sidecar data format, so run the vectors before changing this module. See
//! docs/design/sidecar.md#checksum-implementation and
//! docs/design/packaging.md#crates.

use sha2::{Digest, Sha256};

/// SHA-256 of `data` as the 32 raw digest bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(data));
    out
}

/// SHA-256 of `data` as 64 lowercase hex characters, the form the sidecar's
/// `sha256` field takes.
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
    fn the_published_test_vectors_match() {
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

    // One million 'a' bytes, the long published test vector. Covers many blocks
    // and a length that is an exact multiple of the 64-byte block, so the
    // padding fills a block of its own. Runs in milliseconds.
    #[test]
    fn a_one_million_byte_input_matches_its_published_digest() {
        assert_eq!(
            sha256_hex(&a_times(1_000_000)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    // Lengths chosen around SHA-256's 64-byte block: 55 is the last whose
    // padding still fits one block and 56 the first that needs two, 63, 64 and
    // 65 straddle the block, and 112 leaves a mid-size remainder. Expected
    // digests computed with `shasum -a 256`.
    #[test]
    fn lengths_around_the_padding_boundary_hash_correctly() {
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

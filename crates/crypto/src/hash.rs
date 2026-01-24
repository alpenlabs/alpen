use borsh::BorshSerialize;
use sha2::{Digest, Sha256};
use strata_identifiers::Buf32;

/// Direct untagged hash.
pub fn raw(buf: &[u8]) -> Buf32 {
    Buf32::from(<[u8; 32]>::from(Sha256::digest(buf)))
}

/// Computes a SHA-256 hash over a sequence of byte slices.
///
/// Updates the hasher incrementally for each slice, avoiding the need to
/// concatenate inputs into a single buffer.
pub fn sha256_iter<'a>(bufs: impl IntoIterator<Item = &'a [u8]>) -> Buf32 {
    let mut hasher = Sha256::new();
    for buf in bufs {
        hasher.update(buf);
    }
    Buf32::from(<[u8; 32]>::from(hasher.finalize()))
}

pub fn compute_borsh_hash<T: BorshSerialize>(v: &T) -> Buf32 {
    let mut hasher = Sha256::new();
    v.serialize(&mut hasher).expect("Serialization failed");
    let result = hasher.finalize();
    let arr: [u8; 32] = result.into();
    Buf32::from(arr)
}

/// Implements a double SHA256 (`Sha256d`) hashing function using [RustCrypto's SHA-2 crate](https://github.com/RustCrypto/hashes/tree/master/sha2).
///
/// This implementation is designed to be equivalent to the one found in the
/// [`bitcoin_hashes` crate](https://github.com/rust-bitcoin/rust-bitcoin/blob/master/hashes/src/sha256d.rs)
/// but is built upon the [RustCrypto's SHA-2 crate](https://github.com/RustCrypto/hashes/tree/master/sha2),
/// because it has patches available from both the
/// [Risc0](https://github.com/risc0/RustCrypto-hashes)
/// and [Sp1](https://github.com/sp1-patches/RustCrypto-hashes)
/// crates.
pub fn sha256d(buf: &[u8]) -> Buf32 {
    let mut hasher = Sha256::new();
    hasher.update(buf);
    let result = hasher.finalize_reset();
    hasher.update(result);
    let arr: [u8; 32] = hasher.finalize().into();
    Buf32::from(arr)
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::{sha256d, Hash};
    use rand::{rngs::OsRng, RngCore};
    use strata_identifiers::Buf32;

    use super::sha256d;

    #[test]
    fn test_sha256d_equivalence() {
        let mut array = [0u8; 32];
        OsRng.fill_bytes(&mut array);

        let expected = Buf32::from(sha256d::Hash::hash(&array).to_byte_array());
        let output = sha256d(&array);

        assert_eq!(expected, output);
    }
}

use rkyv::{
    api::high::HighSerializer, rancor::Error as RkyvError, ser::allocator::ArenaHandle,
    util::AlignedVec, Archive, Serialize,
};
use sha2::{Digest, Sha256};
use strata_identifiers::Buf32;

/// Serializer for any type that implements [`Serialize`] as bytes for rkyv.
type HashSerializer<'a> = HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>;

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

/// Computes a SHA-256 hash over the serialized bytes of a type using rkyv.
pub fn compute_rkyv_hash<T>(v: &T) -> Buf32
where
    T: Archive,
    for<'a> T: Serialize<HashSerializer<'a>>,
{
    let bytes = rkyv::to_bytes::<RkyvError>(v).expect("rkyv serialization failed");
    Buf32::from(<[u8; 32]>::from(Sha256::digest(bytes.as_ref())))
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

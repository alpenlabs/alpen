use digest::{generic_array::GenericArray, Digest};
use lazy_static::lazy_static;
use sha2::Sha256;

/// 20 byte hash.
pub type Hash20 = [u8; 20];

/// 32 byte hash.
pub type Hash32 = [u8; 32];

type Tag = [u8; 64];

lazy_static! {
    static ref NODE_TAG_PREFIX: Tag = make_tag(b"node");
    static ref LEAF_TAG_PREFIX: Tag = make_tag(b"leaf");
}

/// Makes a 64 byte tag from a slice, which ideally contains a ASCII string.
fn make_tag(s: &[u8]) -> Tag {
    let raw = Sha256::digest(s);
    let mut buf = [0; 64];
    buf[..32].copy_from_slice(&raw);
    buf[32..].copy_from_slice(&raw);
    buf
}

/// Hash wrapper trait.
pub trait MerkleHash: Copy + Clone + Eq {
    const HASH_LEN: usize;

    /// Returns a zero hash.
    fn zero() -> Self;

    /// Returns if a hash is the zero hash.
    fn is_zero(h: &Self) -> bool;
}

impl<const LEN: usize> MerkleHash for [u8; LEN] {
    const HASH_LEN: usize = LEN;

    fn zero() -> Self {
        [0; LEN]
    }

    fn is_zero(h: &Self) -> bool {
        // Attempt to constant-time eval.
        let sum: u32 = h.iter().map(|v| *v as u32).sum();
        sum == 0
    }
}

/// Generic merkle hashing trait.
pub trait MerkleHasher {
    /// Hash value.
    type Hash: MerkleHash;

    /// Hashes an arbitrary message as leaf data to compute a leaf hash.
    fn hash_leaf(buf: &[u8]) -> Self::Hash;

    /// Hashes a node's left and right children to compute the node's hash.
    fn hash_node(left: Self::Hash, right: Self::Hash) -> Self::Hash;

    /// Convenience function that returns a zero hash from the associated hash
    /// type.
    fn zero_hash() -> Self::Hash {
        <Self::Hash as MerkleHash>::zero()
    }
}

/// Generic impl over [`Digest`] impls, where hash is `[u8; 32]`.
// TODO make this generic over other hash types
impl<D: Digest> MerkleHasher for D {
    type Hash = Hash32;

    fn hash_leaf(buf: &[u8]) -> Self::Hash {
        // This is technically vulnerable to length-extension, but in MMRs that
        // should not matter, and we use the prefix to prevent type confusion.
        let mut context = D::new();
        context.update(*LEAF_TAG_PREFIX);
        context.update(buf);

        let result = context.finalize();
        result
            .as_slice()
            .try_into()
            .expect("mmr: digest output not 32 bytes")
    }

    fn hash_node(left: Self::Hash, right: Self::Hash) -> Self::Hash {
        let mut context = D::new();
        context.update(*NODE_TAG_PREFIX);
        context.update(left);
        context.update(right);

        let result: GenericArray<u8, D::OutputSize> = context.finalize();
        result
            .as_slice()
            .try_into()
            .expect("mmr: digest output not 32 bytes")
    }
}

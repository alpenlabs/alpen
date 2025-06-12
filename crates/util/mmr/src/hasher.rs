use digest::{generic_array::GenericArray, Digest};

/// 20 byte hash.
pub type Hash20 = [u8; 20];

/// 32 byte hash.
pub type Hash32 = [u8; 32];

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

    /// combines the left and Right nodes to form a single Node
    fn hash_node(left: Self::Hash, right: Self::Hash) -> Self::Hash;

    fn zero_hash() -> Self::Hash {
        <Self::Hash as MerkleHash>::zero()
    }
}

/// Generic impl over [`Digest`] impls, where hash is `[u8; 32]`.
// TODO make this generic over other hash types
impl<D: Digest> MerkleHasher for D {
    type Hash = Hash32;

    fn hash_node(left: Self::Hash, right: Self::Hash) -> Self::Hash {
        let mut context = D::new();
        context.update(left);
        context.update(right);

        let result: GenericArray<u8, D::OutputSize> = context.finalize();
        result
            .as_slice()
            .try_into()
            .expect("mmr: digest output not 32 bytes")
    }
}

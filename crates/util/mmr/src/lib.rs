//! Merkle mountain range implementation crate.
pub mod error;
pub mod hasher;

use std::marker::PhantomData;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use error::MerkleError;
use hasher::{Hash, MerkleHasher};

const fn zero() -> Hash {
    [0; 32]
}

fn is_zero(h: Hash) -> bool {
    h.iter().all(|b| *b == 0)
}

/// Compact representation of the MMR that should be borsh serializable easily.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct CompactMmr {
    entries: u64,
    cap_log2: u8,
    roots: Vec<Hash>,
}

#[derive(Clone)]
pub(crate) struct MerkleMr<H: MerkleHasher + Clone> {
    // number of elements inserted into mmr
    #[allow(unused)]
    pub(crate) num: u64,
    // Buffer of all possible peaks in mmr. only some of them will be valid at a time
    pub(crate) peaks: Box<[Hash]>,
    // phantom data for hasher
    pub(crate) hasher: PhantomData<H>,
}

impl<H: MerkleHasher + Clone> MerkleMr<H> {
    #[allow(unused)]
    pub(crate) fn new(cap_log2: usize) -> Self {
        Self {
            num: 0,
            peaks: vec![[0; 32]; cap_log2].into_boxed_slice(),
            hasher: PhantomData,
        }
    }

    #[allow(unused)]
    pub(crate) fn from_compact(compact: &CompactMmr) -> Self {
        // FIXME this is somewhat inefficient, we could consume the vec and just
        // slice out its elements, but this is fine for now
        let mut roots = vec![zero(); compact.cap_log2 as usize];
        let mut at = 0;
        for i in 0..compact.cap_log2 {
            if (compact.entries >> i) & 1 != 0 {
                roots[i as usize] = compact.roots[at as usize];
                at += 1;
            }
        }

        Self {
            num: compact.entries,
            peaks: roots.into(),
            hasher: PhantomData,
        }
    }

    #[allow(unused)]
    pub(crate) fn to_compact(&self) -> CompactMmr {
        CompactMmr {
            entries: self.num,
            cap_log2: self.peaks.len() as u8,
            roots: self
                .peaks
                .iter()
                .filter(|h| !is_zero(**h))
                .copied()
                .collect(),
        }
    }

    pub(crate) fn add_leaf(&mut self, hash_arr: Hash) {
        if self.num == 0 {
            self.peaks[0] = hash_arr;
            self.num += 1;
            return;
        }

        // the number of elements in MMR is also the mask of peaks
        let peak_mask = self.num;

        let mut current_node = hash_arr;
        // we iterate through the height
        let mut current_height = 0;

        while (peak_mask >> current_height) & 1 == 1 {
            let next_node = H::hash_node(self.peaks[current_height], current_node);

            // setting this for debugging purpose
            self.peaks[current_height] = [0; 32];

            current_node = next_node;
            current_height += 1;
        }

        self.peaks[current_height] = current_node;
        self.num += 1;
    }

    #[allow(unused)]
    pub(crate) fn get_single_root(&self) -> Result<Hash, MerkleError> {
        if self.num == 0 {
            return Err(MerkleError::NoElements);
        }
        if !self.num.is_power_of_two() && self.num != 1 {
            return Err(MerkleError::NotPowerOfTwo);
        }

        Ok(self.peaks[(self.num.ilog2()) as usize])
    }

    #[allow(unused)]
    pub(crate) fn add_leaf_updating_proof(
        &mut self,
        next: Hash,
        proof: &MerkleProof<H>,
    ) -> MerkleProof<H> {
        if self.num == 0 {
            self.add_leaf(next);
            return MerkleProof {
                cohashes: vec![],
                index: 0,
                _pd: PhantomData,
            };
        }
        let mut updated_proof = proof.clone();

        let new_leaf_index = self.num;
        let peak_mask = self.num;
        let mut current_node = next;
        let mut current_height = 0;
        while (peak_mask >> current_height) & 1 == 1 {
            let prev_node = self.peaks[current_height];
            let next_node: Hash = H::hash_node(prev_node, current_node);
            let leaf_parent_tree = new_leaf_index >> (current_height + 1);
            self.update_single_proof(
                &mut updated_proof,
                leaf_parent_tree,
                current_height,
                prev_node,
                current_node,
            );

            self.peaks[current_height] = [0; 32];
            current_node = next_node;
            current_height += 1;
        }

        self.peaks[current_height] = current_node;
        self.num += 1;

        updated_proof
    }

    #[allow(unused)]
    fn update_single_proof(
        &mut self,
        proof: &mut MerkleProof<H>,
        leaf_parent_tree: u64,
        current_height: usize,
        prev_node: Hash,
        current_node: Hash,
    ) {
        let proof_index = proof.index;
        let proof_parent_tree = proof_index >> (current_height + 1);
        if leaf_parent_tree == proof_parent_tree {
            if current_height >= proof.cohashes.len() {
                proof.cohashes.resize(current_height + 1, [0; 32]);
            }
            if (proof_index >> current_height) & 1 == 1 {
                proof.cohashes[current_height] = prev_node;
            } else {
                proof.cohashes[current_height] = current_node;
            }
        }
    }

    #[allow(unused)]
    pub(crate) fn add_leaf_updating_proof_list(
        &mut self,
        next: Hash,
        proof_list: &mut [MerkleProof<H>],
    ) -> MerkleProof<H> {
        if self.num == 0 {
            self.add_leaf(next);
            return MerkleProof {
                cohashes: vec![],
                index: 0,
                _pd: PhantomData,
            };
        }
        let mut new_proof = MerkleProof {
            cohashes: vec![],
            index: self.num,
            _pd: PhantomData,
        };

        let new_leaf_index = self.num;
        let peak_mask = self.num;
        let mut current_node = next;
        let mut current_height = 0;
        while (peak_mask >> current_height) & 1 == 1 {
            let prev_node = self.peaks[current_height];
            let next_node: Hash = H::hash_node(prev_node, current_node);
            let leaf_parent_tree = new_leaf_index >> (current_height + 1);
            for proof in proof_list.iter_mut() {
                self.update_single_proof(
                    proof,
                    leaf_parent_tree,
                    current_height,
                    prev_node,
                    current_node,
                );
            }

            self.update_single_proof(
                &mut new_proof,
                leaf_parent_tree,
                current_height,
                prev_node,
                current_node,
            );
            // the peaks value is no longer needed
            self.peaks[current_height] = [0; 32];
            current_node = next_node;
            current_height += 1;
        }

        self.peaks[current_height] = current_node;
        self.num += 1;

        new_proof
    }

    #[allow(unused)]
    pub(crate) fn verify(&self, proof: &MerkleProof<H>, leaf: &Hash) -> bool {
        self.verify_raw(&proof.cohashes, proof.index, leaf)
    }

    fn verify_raw(&self, cohashes: &[Hash], leaf_index: u64, leaf_hash: &Hash) -> bool {
        let root = self.peaks[cohashes.len()];

        if cohashes.is_empty() {
            return root == *leaf_hash;
        }

        let mut cur_hash = *leaf_hash;
        let mut side_flags = leaf_index;

        for cohash in cohashes.iter() {
            let node_hash = if side_flags & 1 == 1 {
                H::hash_node(*cohash, cur_hash)
            } else {
                H::hash_node(cur_hash, *cohash)
            };

            side_flags >>= 1;
            cur_hash = node_hash;
        }
        cur_hash == root
    }

    #[allow(unused)]
    pub(crate) fn gen_proof(
        &self,
        proof_list: &[MerkleProof<H>],
        index: u64,
    ) -> Result<Option<MerkleProof<H>>, MerkleError> {
        if index > self.num {
            return Err(MerkleError::IndexOutOfBounds);
        }

        match proof_list.iter().find(|proof| proof.index == index) {
            Some(proof) => Ok(Some(proof.clone())),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MerkleProof<H>
where
    H: MerkleHasher + Clone,
{
    // sibling hashes required for proof
    pub cohashes: Vec<Hash>,
    // the index of the element for which this proof is for
    pub index: u64,
    pub _pd: PhantomData<H>,
}

impl<H: MerkleHasher + Clone> MerkleProof<H> {
    /// builds the new MerkleProof from the provided Data
    pub const fn from_cohashes(cohashes: Vec<Hash>, index: u64) -> Self {
        Self {
            cohashes,
            index,
            _pd: PhantomData,
        }
    }

    /// verifies the hash against the current proof for given mmr
    #[allow(unused)]
    fn verify_against_mmr(&self, mmr: &MerkleMr<H>, leaf_hash: Hash) -> bool {
        mmr.verify(self, &leaf_hash)
    }
}

#[cfg(test)]
mod test {
    use std::marker::PhantomData;

    use sha2::{Digest, Sha256};

    use super::{hasher::Hash, MerkleMr, MerkleProof};
    use crate::error::MerkleError;

    fn generate_for_n_integers(n: usize) -> (MerkleMr<Sha256>, Vec<MerkleProof<Sha256>>) {
        let mut mmr: MerkleMr<Sha256> = MerkleMr::new(14);

        let mut proof = Vec::new();
        let list_of_hashes = generate_hashes_for_n_integers(n);

        (0..n).for_each(|i| {
            let new_proof = mmr.add_leaf_updating_proof_list(list_of_hashes[i], &mut proof);
            proof.push(new_proof);
        });
        (mmr, proof)
    }

    fn generate_hashes_for_n_integers(n: usize) -> Vec<Hash> {
        (0..n)
            .map(|i| Sha256::digest(i.to_be_bytes()).into())
            .collect::<Vec<Hash>>()
    }

    fn mmr_proof_for_specific_nodes(n: usize, specific_nodes: Vec<u64>) {
        let (mmr, proof_list) = generate_for_n_integers(n);
        let proof: Vec<MerkleProof<Sha256>> = specific_nodes
            .iter()
            .map(|i| {
                mmr.gen_proof(&proof_list, *i)
                    .unwrap()
                    .expect("cannot find proof for the given index")
            })
            .collect();

        let hash: Vec<Hash> = specific_nodes
            .iter()
            .map(|i| Sha256::digest(i.to_be_bytes()).into())
            .collect();

        (0..specific_nodes.len()).for_each(|i| {
            assert!(mmr.verify(&proof[i], &hash[i]));
        });
    }

    #[test]
    fn check_zero_elements() {
        mmr_proof_for_specific_nodes(0, vec![]);
    }

    #[test]
    fn check_two_sibling_leaves() {
        mmr_proof_for_specific_nodes(11, vec![4, 5]);
        mmr_proof_for_specific_nodes(11, vec![5, 6]);
    }

    #[test]
    fn check_single_element() {
        let (mmr, proof_list) = generate_for_n_integers(1);

        let proof = mmr
            .gen_proof(&proof_list, 0)
            .unwrap()
            .expect("Didn't find proof for given index");

        let hash = Sha256::digest(0_usize.to_be_bytes()).into();
        assert!(mmr.verify(&proof, &hash));
    }

    #[test]
    fn check_two_peaks() {
        mmr_proof_for_specific_nodes(3, vec![0, 2]);
    }

    #[test]
    fn check_five_hundred_elements() {
        mmr_proof_for_specific_nodes(500, vec![0, 456]);
    }

    #[test]
    fn check_peak_for_mmr_single_leaf() {
        let hashed1: Hash = Sha256::digest(b"first").into();

        let mut mmr: MerkleMr<Sha256> = MerkleMr::new(14);
        mmr.add_leaf(hashed1);

        assert_eq!(
            mmr.get_single_root(),
            Ok([
                167, 147, 123, 100, 184, 202, 165, 143, 3, 114, 27, 182, 186, 207, 92, 120, 203,
                35, 95, 235, 224, 231, 11, 27, 132, 205, 153, 84, 20, 97, 160, 142
            ])
        );
    }

    #[test]
    fn check_peak_for_mmr_three_leaves() {
        let hashed1: Hash = Sha256::digest(b"first").into();

        let mut mmr: MerkleMr<Sha256> = MerkleMr::new(14);
        mmr.add_leaf(hashed1);
        mmr.add_leaf(hashed1);
        mmr.add_leaf(hashed1);

        assert_eq!(mmr.get_single_root(), Err(MerkleError::NotPowerOfTwo));
    }

    #[test]
    fn check_peak_for_mmr_four_leaves() {
        let hashed1: Hash = Sha256::digest(b"first").into();

        let mut mmr: MerkleMr<Sha256> = MerkleMr::new(14);
        mmr.add_leaf(hashed1);
        mmr.add_leaf(hashed1);
        mmr.add_leaf(hashed1);
        mmr.add_leaf(hashed1);

        assert_eq!(
            mmr.get_single_root(),
            Ok([
                42, 45, 97, 143, 48, 40, 235, 23, 80, 22, 226, 97, 57, 191, 239, 146, 157, 81, 89,
                225, 228, 51, 162, 223, 102, 47, 76, 12, 171, 93, 173, 96
            ])
        );
    }

    #[test]
    fn check_invalid_proof() {
        let (mmr, _) = generate_for_n_integers(5);
        let invalid_proof = MerkleProof::<Sha256> {
            cohashes: vec![],
            index: 6,
            _pd: PhantomData,
        };

        let hash = Sha256::digest(6_usize.to_be_bytes()).into();

        assert!(!mmr.verify(&invalid_proof, &hash));
    }

    #[test]
    fn check_add_node_and_update() {
        let mut mmr: MerkleMr<Sha256> = MerkleMr::new(14);
        let mut proof_list = Vec::new();

        let hashed0: Hash = Sha256::digest(b"first").into();
        let hashed1: Hash = Sha256::digest(b"second").into();
        let hashed2: Hash = Sha256::digest(b"third").into();
        let hashed3: Hash = Sha256::digest(b"fourth").into();
        let hashed4: Hash = Sha256::digest(b"fifth").into();

        let new_proof = mmr.add_leaf_updating_proof_list(hashed0, &mut proof_list);
        proof_list.push(new_proof);

        let new_proof = mmr.add_leaf_updating_proof_list(hashed1, &mut proof_list);
        proof_list.push(new_proof);

        let new_proof = mmr.add_leaf_updating_proof_list(hashed2, &mut proof_list);
        proof_list.push(new_proof);

        let new_proof = mmr.add_leaf_updating_proof_list(hashed3, &mut proof_list);
        proof_list.push(new_proof);

        let new_proof = mmr.add_leaf_updating_proof_list(hashed4, &mut proof_list);
        proof_list.push(new_proof);

        assert!(proof_list[0].verify_against_mmr(&mmr, hashed0));
        assert!(proof_list[1].verify_against_mmr(&mmr, hashed1));
        assert!(proof_list[2].verify_against_mmr(&mmr, hashed2));
        assert!(proof_list[3].verify_against_mmr(&mmr, hashed3));
        assert!(proof_list[4].verify_against_mmr(&mmr, hashed4));
    }

    #[test]
    fn check_compact_and_non_compact() {
        let (mmr, _) = generate_for_n_integers(5);

        let compact_mmr = mmr.to_compact();
        let deserialized_mmr = MerkleMr::<Sha256>::from_compact(&compact_mmr);

        assert_eq!(mmr.num, deserialized_mmr.num);
        assert_eq!(mmr.peaks, deserialized_mmr.peaks);
    }

    #[test]
    fn arbitrary_index_proof() {
        let (mut mmr, _) = generate_for_n_integers(20);
        // update proof for 21st element
        let mut proof: MerkleProof<Sha256> = MerkleProof {
            cohashes: vec![],
            index: 20,
            _pd: PhantomData,
        };

        // add 4 elements into mmr  so 20 + 4 elements
        let num_elems = 4;
        let num_hash = generate_hashes_for_n_integers(num_elems);

        for elem in num_hash.iter().take(num_elems) {
            let new_proof = mmr.add_leaf_updating_proof(*elem, &proof);
            proof = new_proof;
        }

        assert!(proof.verify_against_mmr(&mmr, num_hash[0]));
    }

    #[test]
    fn update_proof_list_from_arbitrary_index() {
        let (mut mmr, _) = generate_for_n_integers(20);
        // update proof for 21st element
        let mut proof_list = Vec::new();

        // add 4 elements into mmr  so 20 + 4 elements
        let num_elems = 4;
        let num_hash = generate_hashes_for_n_integers(num_elems);

        for elem in num_hash.iter().take(num_elems) {
            let new_proof = mmr.add_leaf_updating_proof_list(*elem, &mut proof_list);
            proof_list.push(new_proof);
        }

        for i in 0..num_elems {
            assert!(proof_list[i].verify_against_mmr(&mmr, num_hash[i]));
        }
    }
}

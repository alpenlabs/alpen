//! Utility functions for Bitcoin-related operations.

use strata_identifiers::Buf32;

/// Bitcoin median timestamp window size.
pub const TIMESTAMPS_FOR_MEDIAN: usize = 11;

/// Computes the Merkle cohashes needed for a transaction inclusion proof.
///
/// Given a list of transaction IDs and an index, this function computes the
/// Merkle tree hashes needed to prove that the transaction at `index` is
/// included in the tree.
///
/// Returns a tuple of (cohashes, root) where:
/// - cohashes: The sibling hashes needed for the proof path
/// - root: The Merkle root of the tree
pub fn get_cohashes<T>(ids: &[T], index: u32) -> (Vec<Buf32>, Buf32)
where
    T: Into<Buf32> + Clone,
{
    assert!(
        (index as usize) < ids.len(),
        "The transaction index should be within the txids length"
    );
    let mut curr_level: Vec<Buf32> = ids.iter().cloned().map(Into::into).collect();

    let mut curr_index = index;
    let mut cohashes = vec![];
    while curr_level.len() > 1 {
        let mut next_level = vec![];
        let mut i = 0;
        while i < curr_level.len() {
            let left = curr_level[i];
            let right = if i + 1 < curr_level.len() {
                curr_level[i + 1]
            } else {
                curr_level[i] // duplicate last element if odd
            };

            // Store the cohash (sibling of our path)
            if i == curr_index as usize {
                cohashes.push(right);
            } else if i + 1 == curr_index as usize {
                cohashes.push(left);
            }

            // Compute parent hash
            let mut combined = [0u8; 64];
            combined[..32].copy_from_slice(left.as_ref());
            combined[32..].copy_from_slice(right.as_ref());
            let parent = strata_identifiers::hash::sha256d(&combined);
            next_level.push(parent);

            i += 2;
        }

        curr_index /= 2;
        curr_level = next_level;
    }

    let root = curr_level[0];
    (cohashes, root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cohashes_from_wtxids_idx_2() {
        let input = vec![
            Buf32::from([1; 32]),
            Buf32::from([2; 32]),
            Buf32::from([3; 32]),
            Buf32::from([4; 32]),
            Buf32::from([5; 32]),
        ];

        let (cohashes, _root) = get_cohashes(&input, 2);
        assert_eq!(cohashes.len(), 3);
    }

    #[test]
    fn test_get_cohashes_from_wtxids_idx_5() {
        let input = vec![
            Buf32::from([1; 32]),
            Buf32::from([2; 32]),
            Buf32::from([3; 32]),
            Buf32::from([4; 32]),
            Buf32::from([5; 32]),
            Buf32::from([6; 32]),
        ];

        let (cohashes, _root) = get_cohashes(&input, 5);
        assert_eq!(cohashes.len(), 3);
    }
}

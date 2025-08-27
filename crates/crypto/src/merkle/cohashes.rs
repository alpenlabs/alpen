use strata_primitives::buf::Buf32;

use crate::hashes::sha256d;

/// Generates cohashes and computes the Merkle root for a transaction ID at a specific index
/// within a given slice of elements that can be converted to [`Buf32`].
///
/// This function supports any type that implements the [`Into<Buf32>`] trait, such as
/// [`Txid`s](bitcoin::Txid) or [`Wtxid`s](bitcoin::Wtxid).
///
/// # Parameters
///
/// - `ids`: A slice of ids ([`Txid`s](bitcoin::Txid) or [`Wtxid`s](bitcoin::Wtxid)) that can be
///   converted into [`Buf32`].
/// - `index`: The index of the transaction for which we want the cohashes.
///
/// # Notes
///
/// Cohashes refer to the intermediate hashes (sometimes called "siblings") needed to
/// reconstruct the Merkle path for a given transaction. These intermediate hashes, along with
/// the transaction's hash itself, can be used to compute the Merkle root, thus verifying the
/// transaction’s membership in the Merkle tree.
///
/// # Returns
///
/// - A tuple `(Vec<Buf32>, Buf32)` containing the cohashes and the Merkle root.
///
/// # Panics
///
/// - If the `index` is out of bounds for the `elements` length.
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
    let mut proof = Vec::new();

    while curr_level.len() > 1 {
        let len = curr_level.len();
        if !len.is_multiple_of(2) {
            curr_level.push(curr_level[len - 1]);
        }

        let proof_item_index = if curr_index.is_multiple_of(2) {
            curr_index + 1
        } else {
            curr_index - 1
        };

        let item = curr_level[proof_item_index as usize];
        proof.push(item);

        // construct pairwise hash
        curr_level = curr_level
            .chunks(2)
            .map(|pair| {
                let [a, b] = pair else {
                    panic!("utils: cohash chunk should be a pair");
                };
                let mut arr = [0u8; 64];
                arr[..32].copy_from_slice(a.as_bytes());
                arr[32..].copy_from_slice(b.as_bytes());
                sha256d(&arr)
            })
            .collect::<Vec<_>>();
        curr_index >>= 1;
    }
    (proof, curr_level[0])
}

#[cfg(test)]
mod tests {
    use bitcoin::{consensus::deserialize, Wtxid};

    use super::*;

    fn get_test_wtxids() -> Vec<Wtxid> {
        vec![
            deserialize(&[1; 32]).unwrap(),
            deserialize(&[2; 32]).unwrap(),
            deserialize(&[3; 32]).unwrap(),
            deserialize(&[4; 32]).unwrap(),
            deserialize(&[5; 32]).unwrap(),
            deserialize(&[6; 32]).unwrap(),
            deserialize(&[7; 32]).unwrap(),
        ]
    }

    #[test]
    fn test_get_cohashes_from_wtxids_idx_2() {
        let txids: Vec<Wtxid> = get_test_wtxids();
        let index = 2;

        let (proof, root) = get_cohashes(&txids, index);
        // Validate the proof length
        assert_eq!(proof.len(), 3);

        // Validate the proof contents
        assert_eq!(proof[0].0, [4; 32]);
        assert_eq!(
            proof[1].0,
            [
                57, 206, 32, 190, 222, 130, 201, 107, 137, 8, 190, 196, 161, 87, 176, 156, 84, 155,
                61, 185, 11, 155, 71, 75, 218, 154, 233, 185, 3, 3, 16, 180
            ]
        );
        assert_eq!(
            proof[2].0,
            [
                182, 31, 195, 174, 213, 89, 251, 184, 232, 133, 217, 123, 109, 127, 232, 151, 21,
                83, 204, 182, 115, 231, 30, 116, 89, 113, 163, 62, 104, 190, 1, 213
            ]
        );

        // Validate the root hash
        let expected_root = [
            92, 218, 49, 127, 159, 148, 231, 132, 215, 129, 27, 155, 152, 132, 243, 8, 47, 11, 170,
            252, 138, 147, 167, 219, 111, 149, 245, 126, 165, 46, 146, 105,
        ];
        assert_eq!(root.0, expected_root);
    }

    #[test]
    fn test_get_cohashes_from_wtxids_idx_5() {
        let txids: Vec<Wtxid> = get_test_wtxids();

        let index = 5;

        let (proof, root) = get_cohashes(&txids, index);

        // Validate the proof length
        assert_eq!(proof.len(), 3);

        // Validate the proof contents
        assert_eq!(proof[0].0, [5; 32]);
        assert_eq!(
            proof[1].0,
            [
                166, 91, 23, 162, 124, 131, 204, 95, 164, 84, 106, 176, 191, 145, 187, 217, 223,
                227, 39, 192, 18, 246, 37, 176, 214, 240, 109, 242, 54, 116, 52, 57
            ]
        );
        assert_eq!(
            proof[2].0,
            [
                8, 90, 171, 174, 249, 134, 104, 112, 27, 135, 201, 161, 152, 107, 223, 17, 103, 38,
                169, 148, 152, 2, 50, 107, 105, 137, 86, 151, 212, 232, 200, 18
            ]
        );

        // Validate the root hash
        let expected_root = [
            92, 218, 49, 127, 159, 148, 231, 132, 215, 129, 27, 155, 152, 132, 243, 8, 47, 11, 170,
            252, 138, 147, 167, 219, 111, 149, 245, 126, 165, 46, 146, 105,
        ];
        assert_eq!(root.0, expected_root);
    }
}

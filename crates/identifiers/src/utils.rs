use crate::{hash::sha256d, Buf32};

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
/// transaction's membership in the Merkle tree.
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

//! Bridge Operator Management
//!
//! This module contains types and tables for managing bridge operators

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_crypto::multisig::aggregate_schnorr_keys;
use strata_primitives::{
    buf::Buf32,
    l1::BitcoinXOnlyPublicKey,
    operator::{OperatorIdx, OperatorPubkeys},
    sorted_vec::SortedVec,
};

use super::bitmap::OperatorBitmap;

/// Bridge operator entry containing identification and cryptographic keys.
///
/// Each operator registered in the bridge has:
///
/// - **`idx`** - Unique identifier used to reference the operator globally
/// - **`signing_pk`** - Public key for message signature verification between operators
/// - **`wallet_pk`** - Public key for Bitcoin transaction signatures (MuSig2 compatible)
///
/// # Key Separation Design
///
/// The two separate keys allow for different cryptographic schemes:
/// - Message signing can use a different mechanism than Bitcoin transactions
/// - Currently, only `wallet_pk` is actively used for signatures
///
/// # Bitcoin Compatibility
///
/// The `wallet_pk` follows [BIP 340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki#design)
/// standard, corresponding to a [`PublicKey`](bitcoin::secp256k1::PublicKey) with even parity
/// for compatibility with Bitcoin's Taproot and MuSig2 implementations.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct OperatorEntry {
    /// Global operator index.
    idx: OperatorIdx,

    /// Public key used to verify signed messages from the operator.
    signing_pk: Buf32,

    /// Wallet public key used to compute MuSig2 public key from a set of operators.
    wallet_pk: Buf32,
}

impl PartialOrd for OperatorEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OperatorEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.idx().cmp(&other.idx())
    }
}

impl OperatorEntry {
    /// Returns the unique operator index.
    ///
    /// # Returns
    ///
    /// The [`OperatorIdx`] that uniquely identifies this operator.
    pub fn idx(&self) -> OperatorIdx {
        self.idx
    }

    /// Returns the signing public key for message verification.
    ///
    /// This key is used to verify signed messages between operators in the
    /// bridge communication protocol.
    ///
    /// # Returns
    ///
    /// Reference to the signing public key as [`Buf32`].
    pub fn signing_pk(&self) -> &Buf32 {
        &self.signing_pk
    }

    /// Returns the wallet public key for Bitcoin transactions.
    ///
    /// This key is used in MuSig2 aggregation for Bitcoin transaction signatures
    /// and follows BIP 340 standard for Taproot compatibility.
    ///
    /// # Returns
    ///
    /// Reference to the wallet public key as [`Buf32`].
    pub fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }
}

/// Table for managing registered bridge operators.
///
/// This table maintains all registered operators with efficient lookup and insertion
/// operations. The table automatically assigns unique indices and maintains sorted
/// order for binary search efficiency.
///
/// # Ordering Invariant
///
/// The operators vector **MUST** remain sorted by operator index at all times.
/// This invariant enables O(log n) lookup operations via binary search.
///
/// # Index Management
///
/// The table uses `next_idx` to track and assign operator indices:
///
/// - Indices are assigned sequentially starting from 0
/// - Each new registration increments `next_idx`
/// - Indices are never reused, even after operator exits
///
/// **WARNING**: Since indices are never reused and `OperatorIdx` is `u32`, the table
/// can support at most `u32::MAX` (4,294,967,295) unique operator registrations over
/// its entire lifetime. After reaching this limit, `next_idx` would overflow and the
/// table cannot accept new registrations.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct OperatorTable {
    /// Next unassigned operator index for new registrations.
    next_idx: OperatorIdx,

    /// Vector of registered operators, sorted by operator index.
    ///
    /// **Invariant**: MUST be sorted by `OperatorEntry::idx` field.
    operators: SortedVec<OperatorEntry>,

    /// Bitmap indicating which operators are part of the current N/N multisig.
    ///
    /// Each bit position corresponds to an operator index, where a set bit (1) indicates
    /// the operator at that index is included in the current multisig configuration.
    /// This bitmap is used to efficiently track multisig membership and coordinate
    /// with the aggregated public key for signature operations.
    current_multisig: OperatorBitmap,

    /// Aggregated public key derived from operator wallet keys that are part of the current N/N
    /// multisig.
    ///
    /// This key is computed by aggregating the wallet public keys of only those operators
    /// where `is_in_current_multisig` is true, using the MuSig2 key aggregation protocol.
    /// It serves as the collective public key for multi-signature operations and is used for:
    ///
    /// - Generating deposit addresses for the bridge
    /// - Verifying multi-signatures from the current operator set
    /// - Representing the current N/N multisig set as a single cryptographic entity
    ///
    /// The key is automatically computed when the operator table is created or
    /// updated, ensuring it always reflects the current active multisig participants.
    agg_key: BitcoinXOnlyPublicKey,
}

impl OperatorTable {
    /// Constructs an operator table from a list of operator public keys.
    ///
    /// This method is used during initialization to populate the table with a known set of
    /// operators. Indices are assigned sequentially starting from 0.
    ///
    /// # Parameters
    ///
    /// - `entries` - Slice of [`OperatorPubkeys`] containing signing and wallet keys
    ///
    /// # Returns
    ///
    /// A new [`OperatorTable`] with operators indexed 0, 1, 2, etc.
    ///
    /// # Panics
    ///
    /// Panics if `entries` is empty. At least one operator is required.
    pub fn from_operator_list(entries: &[OperatorPubkeys]) -> Self {
        if entries.is_empty() {
            panic!(
                "Cannot create operator table with empty entries - at least one operator is required"
            );
        }
        let agg_operator_key = aggregate_schnorr_keys(entries.iter().map(|o| o.wallet_pk()))
            .unwrap()
            .into();
        // Create bitmap with all initial operators as active (0, 1, 2, ..., n-1)
        let bitmap = OperatorBitmap::new_with_size(entries.len(), true);
        Self {
            next_idx: entries.len() as OperatorIdx,
            operators: SortedVec::new_unchecked(
                entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| OperatorEntry {
                        idx: i as OperatorIdx,
                        signing_pk: *e.signing_pk(),
                        wallet_pk: *e.wallet_pk(),
                    })
                    .collect(),
            ),
            current_multisig: bitmap,
            agg_key: agg_operator_key,
        }
    }

    /// Returns the number of registered operators.
    pub fn len(&self) -> u32 {
        self.operators.len() as u32
    }

    /// Returns whether the operator table is empty.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    /// Returns a slice of all registered operator entries.
    pub fn operators(&self) -> &[OperatorEntry] {
        self.operators.as_slice()
    }

    /// Returns the aggregated public key of the current active operators
    pub fn agg_key(&self) -> &BitcoinXOnlyPublicKey {
        &self.agg_key
    }

    /// Retrieves an operator entry by its unique index.
    ///
    /// Uses binary search for O(log n) lookup performance.
    ///
    /// # Parameters
    ///
    /// - `idx` - The unique operator index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&OperatorEntry)` if the operator exists
    /// - `None` if no operator with the given index is found
    pub fn get_operator(&self, idx: OperatorIdx) -> Option<&OperatorEntry> {
        self.operators
            .as_slice()
            .binary_search_by_key(&idx, |e| e.idx)
            .ok()
            .map(|i| &self.operators.as_slice()[i])
    }

    /// Returns whether this operator is part of the current N/N multisig set.
    ///
    /// Operators in the current multisig are eligible for new task assignments, while operators
    /// not in the current multisig are preserved in the table but not assigned new tasks.
    ///
    /// # Returns
    ///
    /// `true` if the operator is in the current multisig, `false` otherwise (even if the index is
    /// out-of-bounds).
    pub fn is_in_current_multisig(&self, idx: OperatorIdx) -> bool {
        self.current_multisig.is_active(idx)
    }

    /// Returns a reference to the bitmap of currently active multisig operators.
    ///
    /// The bitmap tracks which operators are part of the current N/N multisig configuration.
    /// This is used for assignment creation and deposit processing.
    pub fn current_multisig(&self) -> &OperatorBitmap {
        &self.current_multisig
    }

    /// Updates the multisig membership status for multiple operators, inserts new operators,
    /// and recalculates the aggregated key.
    ///
    /// # Parameters
    ///
    /// - `updates` - Slice of (operator_index, is_in_multisig) pairs for existing operators
    /// - `inserts` - Slice of new operators to insert (marked as in multisig by default)
    ///
    /// # Processing Order
    ///
    /// Inserts are processed before updates. If an operator index appears in both parameters,
    /// the update will override the insert's `is_in_multisig` value.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The updates would result in no operators being in the multisig
    /// - Sequential operator insertion fails (bitmap index management error)
    /// - `next_idx` overflows `u32::MAX` when inserting new operators (since operator indices are
    ///   never reused, this limits the total number of unique operators that can ever be registered
    ///   to `u32::MAX` or 4,294,967,295 over the bridge's lifetime)
    pub fn update_multisig_and_recalc_key(
        &mut self,
        updates: &[(OperatorIdx, bool)],
        inserts: &[OperatorPubkeys],
    ) {
        // Handle inserts first
        for op_keys in inserts {
            let idx = self.next_idx;
            let entry = OperatorEntry {
                idx,
                signing_pk: *op_keys.signing_pk(),
                wallet_pk: *op_keys.wallet_pk(),
            };

            // SortedVec handles insertion and maintains sorted order
            self.operators.insert(entry);

            // Set new operator as active in bitmap
            self.current_multisig
                .try_set(idx, true)
                .expect("Sequential operator insertion should always succeed");

            self.next_idx += 1;
        }

        // Handle updates by modifying the bitmap directly
        for &(idx, is_in_multisig) in updates {
            // Only update if the operator exists
            if self
                .operators
                .as_slice()
                .binary_search_by_key(&idx, |e| e.idx)
                .is_ok()
            {
                // For existing operators, we can set their status directly
                if (idx as usize) < self.current_multisig.len() {
                    self.current_multisig
                        .try_set(idx, is_in_multisig)
                        .expect("Setting existing operator status should succeed");
                }
            }
        }

        if !updates.is_empty() || !inserts.is_empty() {
            // Recalculate aggregated key based on current multisig members
            let active_keys: Vec<&Buf32> = self
                .current_multisig
                .active_indices()
                .filter_map(|op| self.get_operator(op).map(|entry| entry.wallet_pk()))
                .collect();

            if active_keys.is_empty() {
                panic!("Cannot have empty multisig - at least one operator must be active");
            }

            self.agg_key = aggregate_schnorr_keys(active_keys.into_iter())
                .expect("Failed to generate aggregated key")
                .into();
        }
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{SECP256K1, SecretKey};
    use strata_primitives::operator::OperatorPubkeys;

    use super::*;

    /// Creates test operator pubkeys with randomly generated valid secp256k1 keys
    fn create_test_operator_pubkeys(count: usize) -> Vec<OperatorPubkeys> {
        use bitcoin::secp256k1::rand;
        let mut keys = Vec::with_capacity(count);

        for _ in 0..count {
            // Generate random signing key
            let signing_sk = SecretKey::new(&mut rand::thread_rng());
            let (signing_pk, _) = signing_sk.x_only_public_key(SECP256K1);

            // Generate random wallet key
            let wallet_sk = SecretKey::new(&mut rand::thread_rng());
            let (wallet_pk, _) = wallet_sk.x_only_public_key(SECP256K1);

            keys.push(OperatorPubkeys::new(signing_pk.into(), wallet_pk.into()));
        }

        keys
    }

    #[test]
    #[should_panic(
        expected = "Cannot create operator table with empty entries - at least one operator is required"
    )]
    fn test_operator_table_empty_entries_panics() {
        OperatorTable::from_operator_list(&[]);
    }

    #[test]
    fn test_operator_table_from_operator_list() {
        let operators = create_test_operator_pubkeys(3);
        let table = OperatorTable::from_operator_list(&operators);

        assert_eq!(table.len(), 3);
        assert!(!table.is_empty());
        assert_eq!(table.next_idx, 3);

        // Verify operators are correctly indexed and stored
        for (i, op) in operators.iter().enumerate() {
            let entry = table.get_operator(i as u32).unwrap();
            assert_eq!(entry.idx(), i as u32);
            assert_eq!(entry.signing_pk(), op.signing_pk());
            assert_eq!(entry.wallet_pk(), op.wallet_pk());
            assert!(table.is_in_current_multisig(i as u32));
        }
    }

    #[test]
    fn test_operator_table_insert() {
        let initial_operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&initial_operators);

        let new_operators = create_test_operator_pubkeys(2);
        table.update_multisig_and_recalc_key(&[], &new_operators);

        assert_eq!(table.len(), 3);
        assert_eq!(table.next_idx, 3);

        // Verify inserted operators are correctly stored and in multisig
        for (i, op) in new_operators.iter().enumerate() {
            let idx = (i + 1) as u32;
            let entry = table.get_operator(idx).unwrap();
            assert_eq!(entry.idx(), idx);
            assert_eq!(entry.signing_pk(), op.signing_pk());
            assert_eq!(entry.wallet_pk(), op.wallet_pk());
            assert!(table.is_in_current_multisig(i as u32));
        }
    }

    #[test]
    fn test_operator_table_update_multisig_membership() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially all operators should be in multisig
        assert!(table.is_in_current_multisig(0));
        assert!(table.is_in_current_multisig(1));
        assert!(table.is_in_current_multisig(2));

        // Update multiple operators at once
        let updates = vec![(0, false), (2, false)];
        table.update_multisig_and_recalc_key(&updates, &[]);
        assert!(!table.is_in_current_multisig(0));
        assert!(table.is_in_current_multisig(1)); // unchanged
        assert!(!table.is_in_current_multisig(2));

        // Test with non-existent operator
        let updates = vec![(0, true), (99, false)]; // 99 doesn't exist
        table.update_multisig_and_recalc_key(&updates, &[]);

        // Only existing operator should be updated
        assert!(table.is_in_current_multisig(0));
    }

    #[test]
    fn test_current_multisig_indices() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially, all operators should be in the current multisig set
        let current_indices: Vec<_> = table.current_multisig().active_indices().collect();
        assert_eq!(current_indices, vec![0, 1, 2]);

        // Mark operator 1 as not in current multisig
        table.update_multisig_and_recalc_key(&[(1, false)], &[]);

        // Now only operators 0 and 2 should be in current multisig
        let current_indices: Vec<_> = table.current_multisig().active_indices().collect();
        assert_eq!(current_indices, vec![0, 2]);
    }
}

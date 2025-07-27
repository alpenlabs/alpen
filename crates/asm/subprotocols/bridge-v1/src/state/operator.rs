//! Bridge Operator Management
//!
//! This module contains types and tables for managing bridge operators

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_l1tx::utils::generate_agg_pubkey;
use strata_primitives::{
    bridge::OperatorIdx,
    buf::Buf32,
    l1::XOnlyPk,
    operator::{OperatorKeyProvider, OperatorPubkeys},
};

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
/// standards, corresponding to a [`PublicKey`](bitcoin::secp256k1::PublicKey) with even parity
/// for compatibility with Bitcoin's Taproot and MuSig2 implementations.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct OperatorEntry {
    /// Global operator index.
    idx: OperatorIdx,

    /// Pubkey used to verify signed messages from the operator.
    signing_pk: Buf32,

    /// Wallet pubkey used to compute MuSig2 pubkey from a set of operators.
    wallet_pk: Buf32,

    /// Whether this operator is part of the current N/N multisig set.
    /// Operators not in the current multisig are preserved but not assigned new tasks.
    is_in_current_multisig: bool,
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
    /// and follows BIP 340 standards for Taproot compatibility.
    ///
    /// # Returns
    ///
    /// Reference to the wallet public key as [`Buf32`].
    pub fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }

    /// Returns whether this operator is part of the current N/N multisig set.
    ///
    /// Operators in the current multisig are eligible for new task assignments, while operators
    /// not in the current multisig are preserved in the table but not assigned new tasks.
    ///
    /// # Returns
    ///
    /// `true` if the operator is in the current multisig, `false` otherwise.
    pub fn is_in_current_multisig(&self) -> bool {
        self.is_in_current_multisig
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
/// - `next_idx` tracks the next available operator index for new registrations
/// - Indices are assigned sequentially starting from 0
/// - Once assigned, indices are never reused (no deletion support)
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct OperatorTable {
    /// Next unassigned operator index for new registrations.
    next_idx: OperatorIdx,

    /// Vector of registered operators, sorted by operator index.
    ///
    /// **Invariant**: MUST be sorted by `OperatorEntry::idx` field.
    operators: Vec<OperatorEntry>,

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
    agg_key: XOnlyPk,
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
    ///
    /// # Example
    ///
    /// ```ignore
    /// let operators = vec![
    ///     OperatorPubkeys::new(signing_key1, wallet_key1),
    ///     OperatorPubkeys::new(signing_key2, wallet_key2),
    /// ];
    /// let table = OperatorTable::from_operator_list(&operators);
    /// assert_eq!(table.len(), 2);
    /// ```
    pub fn from_operator_list(entries: &[OperatorPubkeys]) -> Self {
        if entries.is_empty() {
            panic!(
                "Cannot create operator table with empty entries - at least one operator is required"
            );
        }
        let agg_operator_key = generate_agg_pubkey(entries.iter().map(|o| o.wallet_pk()))
            .unwrap()
            .into();
        Self {
            next_idx: entries.len() as OperatorIdx,
            operators: entries
                .iter()
                .enumerate()
                .map(|(i, e)| OperatorEntry {
                    idx: i as OperatorIdx,
                    signing_pk: *e.signing_pk(),
                    wallet_pk: *e.wallet_pk(),
                    is_in_current_multisig: true,
                })
                .collect(),
            agg_key: agg_operator_key,
        }
    }

    /// Validates the operator table's internal invariants.
    ///
    /// Ensures that:
    /// - The operators vector is sorted by operator index
    /// - The `next_idx` is greater than the highest existing operator index
    ///
    /// # Panics
    ///
    /// Panics if any invariant is violated, indicating a bug in the table implementation.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.operators.is_sorted_by_key(|e| e.idx) {
            panic!("bridge_state: operators list not sorted");
        }

        if let Some(e) = self.operators.last()
            && self.next_idx <= e.idx
        {
            panic!("bridge_state: operators next_idx before last entry");
        }
    }

    /// Returns the number of registered operators.
    ///
    /// # Returns
    ///
    /// The total count of operators in the table as [`u32`].
    pub fn len(&self) -> u32 {
        self.operators.len() as u32
    }

    /// Returns whether the operator table is empty.
    ///
    /// In practice, this will typically return `false` since bridge operation
    /// requires at least one registered operator.
    ///
    /// # Returns
    ///
    /// `true` if no operators are registered, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    /// Returns a slice of all registered operator entries.
    ///
    /// The entries are guaranteed to be sorted by operator index.
    ///
    /// # Returns
    ///
    /// Slice reference to all [`OperatorEntry`] instances in the table.
    pub fn operators(&self) -> &[OperatorEntry] {
        &self.operators
    }

    pub fn agg_key(&self) -> &XOnlyPk {
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
    pub fn get_operator(&self, idx: u32) -> Option<&OperatorEntry> {
        self.operators
            .binary_search_by_key(&idx, |e| e.idx)
            .ok()
            .map(|i| &self.operators[i])
    }

    /// Retrieves an operator entry by its position in the internal vector.
    ///
    /// This method accesses operators by their storage position rather than their
    /// logical index. Useful for iteration or when the position is known.
    ///
    /// # Parameters
    ///
    /// - `pos` - The position in the internal vector (0-based)
    ///
    /// # Returns
    ///
    /// - `Some(&OperatorEntry)` if the position is valid
    /// - `None` if the position is out of bounds
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&OperatorEntry> {
        self.operators.get(pos as usize)
    }

    /// Returns an iterator over all operator indices.
    ///
    /// The indices are returned in sorted order due to the table's invariant.
    ///
    /// # Returns
    ///
    /// Iterator yielding each registered operator's [`OperatorIdx`].
    pub fn indices(&self) -> impl Iterator<Item = OperatorIdx> + '_ {
        self.operators.iter().map(|operator| operator.idx)
    }

    /// Returns an iterator over indices of operators in the current N/N multisig.
    ///
    /// Only returns indices of operators where `is_in_current_multisig` is `true`.
    /// This is used for assignment creation and deposit processing.
    ///
    /// # Returns
    ///
    /// Iterator yielding [`OperatorIdx`] for operators in the current multisig.
    pub fn current_multisig_indices(&self) -> impl Iterator<Item = OperatorIdx> + '_ {
        self.operators
            .iter()
            .filter(|operator| operator.is_in_current_multisig)
            .map(|operator| operator.idx)
    }

    /// Updates the multisig membership status for multiple operators, inserts new operators,
    /// and recalculates the aggregated key.
    ///
    /// This is the central method for updating multisig membership. All other methods that modify
    /// the multisig set should call this method internally to ensure the aggregated key is
    /// correctly recalculated.
    ///
    /// # Parameters
    ///
    /// - `updates` - Slice of (operator_index, is_in_multisig) pairs for existing operators
    /// - `inserts` - Slice of new operators to insert (marked as in multisig by default)
    ///
    /// # Panics
    ///
    /// Panics if the updates would result in no operators being in the multisig.
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
                is_in_current_multisig: true,
            };

            // Insert in correct position to maintain sorted order
            let insert_pos = self
                .operators
                .binary_search_by_key(&idx, |e| e.idx)
                .unwrap_or_else(|pos| pos);
            self.operators.insert(insert_pos, entry);

            self.next_idx += 1;
        }

        // Handle updates
        for &(idx, is_in_multisig) in updates {
            if let Ok(pos) = self.operators.binary_search_by_key(&idx, |e| e.idx) {
                self.operators[pos].is_in_current_multisig = is_in_multisig;
            }
        }

        if !updates.is_empty() || !inserts.is_empty() {
            // Recalculate aggregated key based on current multisig members
            let active_keys: Vec<&Buf32> = self
                .operators
                .iter()
                .filter(|op| op.is_in_current_multisig)
                .map(|op| &op.wallet_pk)
                .collect();

            if active_keys.is_empty() {
                panic!("Cannot have empty multisig - at least one operator must be active");
            }

            self.agg_key = generate_agg_pubkey(active_keys.into_iter())
                .expect("Failed to generate aggregated key")
                .into();
        }
    }
}

impl OperatorKeyProvider for OperatorTable {
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32> {
        // TODO: use the `signing_pk` here if we decide to use a different signing scheme for
        // signing messages.
        self.get_operator(idx).map(|ent| ent.wallet_pk)
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
    fn test_operator_entry_getters() {
        let (signing_pk, wallet_pk) = create_test_operator_pubkeys(1)[0].clone().into_parts();

        let entry = OperatorEntry {
            idx: 5,
            signing_pk,
            wallet_pk,
            is_in_current_multisig: true,
        };

        assert_eq!(entry.idx(), 5);
        assert_eq!(entry.signing_pk(), &signing_pk);
        assert_eq!(entry.wallet_pk(), &wallet_pk);
        assert!(entry.is_in_current_multisig());
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

        // Check first operator
        let op0 = table.get_operator(0).unwrap();
        assert_eq!(op0.idx(), 0);
        assert_eq!(op0.signing_pk(), operators[0].signing_pk());
        assert_eq!(op0.wallet_pk(), operators[0].wallet_pk());

        // Check second operator
        let op1 = table.get_operator(1).unwrap();
        assert_eq!(op1.idx(), 1);
        assert_eq!(op1.signing_pk(), operators[1].signing_pk());
        assert_eq!(op1.wallet_pk(), operators[1].wallet_pk());
    }

    #[test]
    fn test_operator_table_insert() {
        // Start with one operator since we can't have empty table
        let initial_operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&initial_operators);

        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx, 1);

        // Insert new operators with valid keys
        let new_operators = create_test_operator_pubkeys(2);
        let start_idx = table.next_idx;
        table.update_multisig_and_recalc_key(&[], &new_operators);
        let assigned_indices: Vec<OperatorIdx> =
            (start_idx..start_idx + new_operators.len() as OperatorIdx).collect();

        assert_eq!(table.len(), 3); // 1 initial + 2 new
        assert_eq!(table.next_idx, 3);
        assert_eq!(assigned_indices, vec![1, 2]);

        // Check the first inserted operator (index 1)
        let op1 = table.get_operator(1).unwrap();
        assert_eq!(op1.idx(), 1);
        assert_eq!(op1.signing_pk(), new_operators[0].signing_pk());
        assert_eq!(op1.wallet_pk(), new_operators[0].wallet_pk());

        // Check the second inserted operator (index 2)
        let op2 = table.get_operator(2).unwrap();
        assert_eq!(op2.idx(), 2);
        assert_eq!(op2.signing_pk(), new_operators[1].signing_pk());
        assert_eq!(op2.wallet_pk(), new_operators[1].wallet_pk());

        // All new operators should be in the current multisig by default
        assert!(op1.is_in_current_multisig());
        assert!(op2.is_in_current_multisig());
    }

    #[test]
    fn test_operator_table_update_multisig_membership() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially all operators should be in multisig
        assert!(table.get_operator(0).unwrap().is_in_current_multisig());
        assert!(table.get_operator(1).unwrap().is_in_current_multisig());
        assert!(table.get_operator(2).unwrap().is_in_current_multisig());

        // Update multiple operators at once
        let updates = vec![(0, false), (2, false)];
        table.update_multisig_and_recalc_key(&updates, &[]);
        assert!(!table.get_operator(0).unwrap().is_in_current_multisig());
        assert!(table.get_operator(1).unwrap().is_in_current_multisig()); // unchanged
        assert!(!table.get_operator(2).unwrap().is_in_current_multisig());

        // Test with non-existent operator
        let updates = vec![(0, true), (99, false)]; // 99 doesn't exist
        table.update_multisig_and_recalc_key(&updates, &[]);

        // Only existing operator should be updated
        assert!(table.get_operator(0).unwrap().is_in_current_multisig());
    }

    #[test]
    fn test_operator_table_remove_from_active_set() {
        let operators = create_test_operator_pubkeys(4);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Remove operators 1 and 3 from active set
        let updates: Vec<(OperatorIdx, bool)> = vec![(1, false), (3, false)];
        table.update_multisig_and_recalc_key(&updates, &[]);
        assert!(table.get_operator(0).unwrap().is_in_current_multisig());
        assert!(!table.get_operator(1).unwrap().is_in_current_multisig());
        assert!(table.get_operator(2).unwrap().is_in_current_multisig());
        assert!(!table.get_operator(3).unwrap().is_in_current_multisig());

        // Test with non-existent operators
        let updates: Vec<(OperatorIdx, bool)> = vec![(1, false), (99, false)]; // 1 already inactive, 99 doesn't exist
        table.update_multisig_and_recalc_key(&updates, &[]);

        // Operator 1 should still be inactive
        assert!(!table.get_operator(1).unwrap().is_in_current_multisig());
    }

    #[test]
    fn test_empty_insert() {
        let operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Test inserting empty slice
        table.update_multisig_and_recalc_key(&[], &[]);
        assert_eq!(table.len(), 1); // No change
        assert_eq!(table.next_idx, 1); // No change
    }

    #[test]
    fn test_operator_table_get_operator() {
        let operators = create_test_operator_pubkeys(2);
        let table = OperatorTable::from_operator_list(&operators);

        // Test existing operators
        assert!(table.get_operator(0).is_some());
        assert!(table.get_operator(1).is_some());

        // Test non-existing operator
        assert!(table.get_operator(2).is_none());
        assert!(table.get_operator(100).is_none());
    }

    #[test]
    fn test_operator_table_get_entry_at_pos() {
        let operators = create_test_operator_pubkeys(2);
        let table = OperatorTable::from_operator_list(&operators);

        // Test valid positions
        let op0 = table.get_entry_at_pos(0).unwrap();
        assert_eq!(op0.idx(), 0);

        let op1 = table.get_entry_at_pos(1).unwrap();
        assert_eq!(op1.idx(), 1);

        // Test invalid positions
        assert!(table.get_entry_at_pos(2).is_none());
        assert!(table.get_entry_at_pos(100).is_none());
    }

    #[test]
    fn test_operator_table_indices() {
        let operators = create_test_operator_pubkeys(3);
        let table = OperatorTable::from_operator_list(&operators);

        let indices: Vec<_> = table.indices().collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_operator_key_provider() {
        let operators = create_test_operator_pubkeys(2);
        let table = OperatorTable::from_operator_list(&operators);

        // Test existing operator
        let signing_pk = table.get_operator_signing_pk(0).unwrap();
        assert_eq!(signing_pk, *operators[0].wallet_pk()); // Returns wallet_pk

        let signing_pk2 = table.get_operator_signing_pk(1).unwrap();
        assert_eq!(signing_pk2, *operators[1].wallet_pk()); // Returns wallet_pk

        // Test non-existing operator
        assert!(table.get_operator_signing_pk(2).is_none());
    }

    #[test]
    fn test_operator_table_sanity_check() {
        // Test with one operator (minimum required)
        let operators = create_test_operator_pubkeys(1);
        let table = OperatorTable::from_operator_list(&operators);
        table.sanity_check(); // Should not panic on single operator table

        let operators = create_test_operator_pubkeys(2);
        let table = OperatorTable::from_operator_list(&operators);
        table.sanity_check(); // Should not panic on valid table
    }

    #[test]
    fn test_current_multisig_indices() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially, all operators should be in the current multisig set
        let current_indices: Vec<_> = table.current_multisig_indices().collect();
        assert_eq!(current_indices, vec![0, 1, 2]);

        // Mark operator 1 as not in current multisig
        table.update_multisig_and_recalc_key(&[(1, false)], &[]);

        // Now only operators 0 and 2 should be in current multisig
        let current_indices: Vec<_> = table.current_multisig_indices().collect();
        assert_eq!(current_indices, vec![0, 2]);
    }

    #[test]
    fn test_set_multisig_membership() {
        let operators = create_test_operator_pubkeys(2);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially, both operators should be in current multisig
        assert!(table.get_operator(0).unwrap().is_in_current_multisig());
        assert!(table.get_operator(1).unwrap().is_in_current_multisig());

        // Remove operator 0 from current multisig
        table.update_multisig_and_recalc_key(&[(0, false)], &[]);
        assert!(!table.get_operator(0).unwrap().is_in_current_multisig());
        assert!(table.get_operator(1).unwrap().is_in_current_multisig());

        // Try to update non-existent operator (should be ignored)
        table.update_multisig_and_recalc_key(&[(99, false)], &[]);

        // Add operator 0 back to current multisig
        table.update_multisig_and_recalc_key(&[(0, true)], &[]);
        assert!(table.get_operator(0).unwrap().is_in_current_multisig());
    }
}

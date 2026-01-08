//! Bridge Operator Management
//!
//! This module contains types and tables for managing bridge operators

use std::cmp;

use bitcoin::{ScriptBuf, secp256k1::SECP256K1};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_bridge_types::OperatorIdx;
use strata_btc_types::BitcoinScriptBuf;
use strata_crypto::{EvenPublicKey, aggregate_schnorr_keys};
use strata_primitives::{buf::Buf32, l1::BitcoinXOnlyPublicKey, sorted_vec::SortedVec};

use super::bitmap::OperatorBitmap;

/// Bridge operator entry containing identification and cryptographic keys.
///
/// Each operator registered in the bridge has:
///
/// - **`idx`** - Unique identifier used to reference the operator globally
/// - **`musig2_pk`** - Public key for Bitcoin transaction signatures (MuSig2 compatible)
///
/// # Bitcoin Compatibility
///
/// The `musig2_pk` follows [BIP 340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki#design)
/// standard, corresponding to a [`PublicKey`](bitcoin::secp256k1::PublicKey) with even parity
/// for compatibility with Bitcoin's Taproot and MuSig2 implementations.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct OperatorEntry {
    /// Global operator index.
    idx: OperatorIdx,

    /// Public key used to compute MuSig2 public key from a set of operators.
    musig2_pk: EvenPublicKey,
}

impl PartialOrd for OperatorEntry {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OperatorEntry {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
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

    /// Returns the MuSig2 public key for Bitcoin transactions.
    ///
    /// This key is used in MuSig2 aggregation for Bitcoin transaction signatures
    /// and follows BIP 340 standard for Taproot compatibility.
    ///
    /// # Returns
    ///
    /// Reference to the MuSig2 public key as [`EvenPublicKey`].
    pub fn musig2_pk(&self) -> &EvenPublicKey {
        &self.musig2_pk
    }
}

/// Builds a key-path-only P2TR script for the provided aggregated operator key.
pub(crate) fn build_nn_script(agg_key: &BitcoinXOnlyPublicKey) -> BitcoinScriptBuf {
    BitcoinScriptBuf::from(ScriptBuf::new_p2tr(
        SECP256K1,
        agg_key.to_xonly_public_key(),
        None,
    ))
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

    /// Bitmap indicating which operators are currently active in the N/N multisig.
    ///
    /// Each bit position corresponds to an operator index, where a set bit (1) indicates
    /// the operator at that index is currently active in the multisig configuration.
    /// This bitmap is used to efficiently track active operator membership and coordinate
    /// with the aggregated public key for signature operations.
    active_operators: OperatorBitmap,

    /// Aggregated public key derived from operator MuSig2 keys that are currently active in the
    /// N/N multisig.
    ///
    /// This key is computed by aggregating the MuSig2 public keys of only those operators
    /// marked as active in the `active_operators` bitmap, using the MuSig2 key aggregation
    /// protocol. It serves as the collective public key for multi-signature operations and is
    /// used for:
    ///
    /// - Generating deposit addresses for the bridge
    /// - Verifying multi-signatures from the current operator set
    /// - Representing the current N/N multisig set as a single cryptographic entity
    ///
    /// The key is automatically computed when the operator table is created or
    /// updated, ensuring it always reflects the current active multisig participants.
    agg_key: BitcoinXOnlyPublicKey,

    /// Historical N/N multisig scripts from previous operator set configurations.
    ///
    /// This vector tracks all P2TR scripts that represented the bridge across membership changes
    /// due to operator entries/exits. Each script is a key-path-only P2TR output (merkle root =
    /// None) constructed from the aggregated public key of the operator set at that time.
    ///
    /// By storing the ScriptBuf directly instead of just keys, we avoid recomputing P2TR scripts
    /// during validation, improving performance.
    historical_nn_scripts: Vec<BitcoinScriptBuf>,
}

impl OperatorTable {
    /// Bootstraps an operator table with an initial active operator set.
    ///
    /// Every provided key is added and marked active, skipping any duplicates. The aggregated
    /// MuSig2 key and the first N/N script are computed via the normal membership change flow so
    /// script history starts with this initial configuration.
    ///
    /// Indices are assigned sequentially starting from 0.
    ///
    /// # Parameters
    ///
    /// - `operators` - Initial set of [`EvenPublicKey`] MuSig2 keys. Duplicate keys are ignored.
    ///
    /// # Panics
    ///
    /// Panics if `operators` is empty. At least one operator is required.
    pub fn from_operator_list(operators: &[EvenPublicKey]) -> Self {
        if operators.is_empty() {
            panic!(
                "Cannot create operator table with empty entries - at least one operator is required"
            );
        }

        // Placeholder so the table is fully initialized before we compute the real aggregated key.
        let placeholder_agg_key = BitcoinXOnlyPublicKey::new([1u8; 32].into()).unwrap();

        let mut table = Self {
            next_idx: 0,
            operators: SortedVec::new_empty(),
            active_operators: OperatorBitmap::new_empty(),
            agg_key: placeholder_agg_key,
            historical_nn_scripts: Vec::new(),
        };

        // Reuse membership change flow to handle deduplication and seed script history.
        table.apply_membership_changes(operators, &[]);

        table
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

    /// Returns the aggregated public key of the current active operators.
    ///
    /// This key is computed by aggregating the MuSig2 public keys of all active operators.
    pub fn agg_key(&self) -> &BitcoinXOnlyPublicKey {
        &self.agg_key
    }

    /// Returns an iterator over all stored N/N multisig scripts in chronological order.
    ///
    /// The scripts represent past N/N multisig configurations (with the last entry always
    /// corresponding to the current operator set) and are used to validate slash transactions that
    /// reference stake connectors from those historical operator sets.
    pub fn historical_nn_scripts(&self) -> impl Iterator<Item = &ScriptBuf> {
        self.historical_nn_scripts.iter().map(|s| s.inner())
    }

    /// Returns the current N/N multisig script for the active operator set.
    ///
    /// The latest script is stored as the last entry in `historical_nn_scripts` and is reused for
    /// validating new slash transactions and stake connectors without recomputing.
    pub fn current_nn_script(&self) -> &ScriptBuf {
        self.historical_nn_scripts
            .last()
            .expect("N/N script history should never be empty")
            .inner()
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

    /// Returns whether this operator is currently active in the N/N multisig set.
    ///
    /// Active operators are eligible for new task assignments, while inactive operators
    /// are preserved in the table but not assigned new tasks.
    ///
    /// # Returns
    ///
    /// `true` if the operator is active, `false` otherwise (even if the index is
    /// out-of-bounds).
    pub fn is_in_current_multisig(&self, idx: OperatorIdx) -> bool {
        self.active_operators.is_active(idx)
    }

    /// Returns a reference to the bitmap of currently active operators.
    ///
    /// The bitmap tracks which operators are currently active in the N/N multisig configuration.
    /// This is used for assignment creation and deposit processing.
    pub fn current_multisig(&self) -> &OperatorBitmap {
        &self.active_operators
    }

    /// Atomically applies membership changes by adding new operators and removing existing ones,
    /// then recalculates the aggregated key.
    ///
    /// After recalculating, the new N/N script is appended to `historical_nn_scripts` so the latest
    /// script remains accessible while older entries continue to support validation for previous
    /// operator configurations.
    ///
    /// # Processing Order
    ///
    /// Additions are processed before removals. If an operator index appears in both parameters,
    /// the removal will override the addition's `is_active` value.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The changes would result in no active operators
    /// - Sequential operator insertion fails (bitmap index management error)
    /// - `next_idx` overflows `u32::MAX` when inserting new operators (since operator indices are
    ///   never reused, this limits the total number of unique operators that can ever be registered
    ///   to `u32::MAX` or 4,294,967,295 over the bridge's lifetime)
    pub fn apply_membership_changes(
        &mut self,
        add_members: &[EvenPublicKey],
        remove_members: &[OperatorIdx],
    ) {
        self.add_operators(add_members);
        self.remove_operators(remove_members);

        let did_change = !remove_members.is_empty() || !add_members.is_empty();
        if did_change {
            self.calculate_aggregated_key();
            self.historical_nn_scripts
                .push(build_nn_script(&self.agg_key));
        }
    }

    /// Adds new operators to the table and marks them as active.
    ///
    /// # Duplicate Keys
    ///
    /// Duplicate public keys are **NOT** allowed. If an operator already exists in the table,
    /// the duplicate addition is ignored, and the existing operator remains unaffected.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - Sequential operator insertion fails (bitmap index management error)
    /// - `next_idx` overflows `u32::MAX`
    fn add_operators(&mut self, operators: &[EvenPublicKey]) {
        for musig2_pk in operators {
            // Check if it already exists in the table (which handles both existing operators
            // and internal duplicates in the input list, as the first occurrence is added)
            if self.operators.iter().any(|op| op.musig2_pk() == musig2_pk) {
                eprintln!("Skipping duplicate operator: {:?}", musig2_pk);
                continue;
            }

            let idx = self.next_idx;
            let entry = OperatorEntry {
                idx,
                musig2_pk: *musig2_pk,
            };

            // SortedVec handles insertion and maintains sorted order
            self.operators.insert(entry);

            // Set new operator as active in bitmap
            self.active_operators
                .try_set(idx, true)
                .expect("Sequential operator insertion should always succeed");

            self.next_idx += 1;
        }
    }

    /// Deactivates existing operators by their indices.
    fn remove_operators(&mut self, indices: &[OperatorIdx]) {
        for &idx in indices {
            // Only update if the operator exists
            if self
                .operators
                .as_slice()
                .binary_search_by_key(&idx, |e| e.idx)
                .is_ok()
            {
                // For existing operators, we can set their status directly
                if (idx as usize) < self.active_operators.len() {
                    self.active_operators
                        .try_set(idx, false)
                        .expect("Setting existing operator status should succeed");
                }
            }
        }
    }

    /// Calculates the aggregated key based on currently active operators.
    ///
    /// # Panics
    ///
    /// Panics if there are no active operators.
    fn calculate_aggregated_key(&mut self) {
        let active_keys: Vec<Buf32> = self
            .active_operators
            .active_indices()
            .filter_map(|op| {
                self.get_operator(op)
                    .map(|entry| Buf32::from(entry.musig2_pk().x_only_public_key().0.serialize()))
            })
            .collect();

        if active_keys.is_empty() {
            panic!("Cannot have empty multisig - at least one operator must be active");
        }

        self.agg_key = aggregate_schnorr_keys(active_keys.iter())
            .expect("Failed to generate aggregated key")
            .into();
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{SECP256K1, SecretKey};

    use super::*;

    /// Creates test operator MuSig2 public keys with randomly generated valid secp256k1 keys
    fn create_test_operator_pubkeys(count: usize) -> Vec<EvenPublicKey> {
        use bitcoin::secp256k1::rand;
        let mut keys = Vec::with_capacity(count);

        for _ in 0..count {
            // Generate random MuSig2 key
            let sk = SecretKey::new(&mut rand::thread_rng());
            let pk = sk.public_key(SECP256K1);
            keys.push(EvenPublicKey::from(pk));
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

        // Verify all operators are present in the table
        // Note: Indices may not match input order due to HashSet deduplication
        for op_pk in &operators {
            let found = table
                .operators()
                .iter()
                .any(|entry| entry.musig2_pk() == op_pk);
            assert!(found, "Operator {:?} not found in table", op_pk);
        }

        // precise index check is no longer valid as order is not guaranteed
        for entry in table.operators() {
            assert!(table.is_in_current_multisig(entry.idx()));
        }
    }

    #[test]
    fn test_operator_table_insert() {
        let initial_operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&initial_operators);

        let new_operators = create_test_operator_pubkeys(2);
        table.apply_membership_changes(&new_operators, &[]);

        assert_eq!(table.len(), 3);
        assert_eq!(table.next_idx, 3);

        // Verify inserted operators are correctly stored and active
        for (i, op_pk) in new_operators.iter().enumerate() {
            let idx = (i + 1) as u32;
            let entry = table.get_operator(idx).unwrap();
            assert_eq!(entry.idx(), idx);
            assert_eq!(entry.musig2_pk(), op_pk);
            assert!(table.is_in_current_multisig(idx));
        }
    }

    #[test]
    fn test_operator_table_insert_duplicate() {
        let initial_operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&initial_operators);

        // Try to add the same operator again - should be silently ignored
        table.apply_membership_changes(&initial_operators, &[]);

        // Check that state hasn't changed (still 1 operator)
        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx, 1);
    }

    #[test]
    fn test_operator_table_create_with_duplicate_deduplicates() {
        let mut operators = create_test_operator_pubkeys(2);
        operators[1] = operators[0]; // Duplicate key

        let table = OperatorTable::from_operator_list(&operators);

        // Should have deduplicated to just 1 operator
        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx, 1);
        assert_eq!(*table.get_operator(0).unwrap().musig2_pk(), operators[0]);
    }

    #[test]
    fn test_operator_table_add_internal_duplicates() {
        let initial_operators = create_test_operator_pubkeys(1);
        let mut table = OperatorTable::from_operator_list(&initial_operators);

        let new_operators = create_test_operator_pubkeys(1);
        // Try to add the same new operator twice in the same batch
        let duplicates = vec![new_operators[0], new_operators[0]];

        table.apply_membership_changes(&duplicates, &[]);

        // Should have only added one new operator
        assert_eq!(table.len(), 2);
        assert_eq!(table.next_idx, 2);
    }

    #[test]
    fn test_operator_table_update_active_status() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially all operators should be active
        assert!(table.is_in_current_multisig(0));
        assert!(table.is_in_current_multisig(1));
        assert!(table.is_in_current_multisig(2));

        // Update multiple operators at once
        let removals = vec![0, 2];
        table.apply_membership_changes(&[], &removals);
        assert!(!table.is_in_current_multisig(0));
        assert!(table.is_in_current_multisig(1)); // unchanged
        assert!(!table.is_in_current_multisig(2));

        // Test re-adding operator 0
        let additions = vec![0];
        table.apply_membership_changes(&[], &additions);

        // Operator 0 should remain inactive (it was already added)
        assert!(!table.is_in_current_multisig(0));
    }

    #[test]
    fn test_active_operators_indices() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        // Initially, all operators should be active
        let active_indices: Vec<_> = table.current_multisig().active_indices().collect();
        assert_eq!(active_indices, vec![0, 1, 2]);

        // Mark operator 1 as inactive
        table.apply_membership_changes(&[], &[1]);

        // Now only operators 0 and 2 should be active
        let active_indices: Vec<_> = table.current_multisig().active_indices().collect();
        assert_eq!(active_indices, vec![0, 2]);
    }

    #[test]
    fn test_historical_nn_scripts_tracking() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        let historical_scripts: Vec<_> = table.historical_nn_scripts().collect();
        assert_eq!(historical_scripts.len(), 1);
        let initial_script = table.current_nn_script().clone();
        assert_eq!(historical_scripts[0], &initial_script);

        table.apply_membership_changes(&[], &[0]);

        let second_script = table.current_nn_script().clone();
        assert_ne!(second_script, initial_script);

        let historical_scripts: Vec<_> = table.historical_nn_scripts().collect();
        assert_eq!(historical_scripts.len(), 2);
        assert_eq!(historical_scripts[0], &initial_script);
        assert_eq!(historical_scripts[1], &second_script);

        table.apply_membership_changes(&[], &[1]);

        let third_script = table.current_nn_script().clone();
        assert_ne!(third_script, second_script);

        let historical_scripts: Vec<_> = table.historical_nn_scripts().collect();
        assert_eq!(historical_scripts.len(), 3);
        assert_eq!(historical_scripts[0], &initial_script);
        assert_eq!(historical_scripts[1], &second_script);
        assert_eq!(historical_scripts[2], &third_script);

        assert_ne!(initial_script, second_script);
        assert_ne!(second_script, third_script);
        assert_ne!(initial_script, third_script);
    }

    #[test]
    fn test_historical_scripts_on_additions() {
        let operators = create_test_operator_pubkeys(3);
        let mut table = OperatorTable::from_operator_list(&operators);

        let historical_scripts: Vec<_> = table.historical_nn_scripts().collect();
        assert_eq!(historical_scripts.len(), 1);
        let initial_script = table.current_nn_script().clone();

        let new_operators = create_test_operator_pubkeys(2);
        table.apply_membership_changes(&new_operators, &[]);

        let new_script = table.current_nn_script().clone();
        assert_ne!(new_script, initial_script);

        let historical_scripts: Vec<_> = table.historical_nn_scripts().collect();
        assert_eq!(historical_scripts.len(), 2);
        assert_eq!(historical_scripts[0], &initial_script);
        assert_eq!(historical_scripts[1], &new_script);
    }
}

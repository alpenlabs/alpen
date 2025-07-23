//! Bridge Operator Management
//!
//! This module contains types and tables for managing bridge operators

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::OperatorIdx,
    buf::Buf32,
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
}

impl OperatorTable {
    /// Creates a new empty operator table.
    ///
    /// Initializes the table with no operators and `next_idx` set to 0,
    /// ready for operator registrations.
    ///
    /// # Returns
    ///
    /// A new empty [`OperatorTable`].
    pub fn new_empty() -> Self {
        Self {
            next_idx: 0,
            operators: Vec::new(),
        }
    }

    /// Constructs an operator table from a list of operator public keys.
    ///
    /// This convenience method is used during initialization to populate the table
    /// with a known set of operators. Indices are assigned sequentially starting from 0.
    ///
    /// # Parameters
    ///
    /// - `entries` - Slice of [`OperatorPubkeys`] containing signing and wallet keys
    ///
    /// # Returns
    ///
    /// A new [`OperatorTable`] with operators indexed 0, 1, 2, etc.
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
        Self {
            next_idx: entries.len() as OperatorIdx,
            operators: entries
                .iter()
                .enumerate()
                .map(|(i, e)| OperatorEntry {
                    idx: i as OperatorIdx,
                    signing_pk: *e.signing_pk(),
                    wallet_pk: *e.wallet_pk(),
                })
                .collect(),
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

    /// Registers a new operator with the provided public keys.
    ///
    /// The operator is assigned the next available index and appended to the table.
    /// Since indices are assigned sequentially, this maintains the sorted order invariant.
    ///
    /// # Parameters
    ///
    /// - `signing_pk` - Public key for message signature verification
    /// - `wallet_pk` - Public key for Bitcoin transaction signatures (MuSig2 compatible)
    pub fn insert(&mut self, signing_pk: Buf32, wallet_pk: Buf32) {
        let entry = OperatorEntry {
            idx: {
                let idx = self.next_idx;
                self.next_idx += 1;
                idx
            },
            signing_pk,
            wallet_pk,
        };
        self.operators.push(entry);
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
    use strata_primitives::operator::OperatorPubkeys;

    use super::*;

    fn create_test_buf32(value: u8) -> Buf32 {
        let mut buf = [0u8; 32];
        buf[0] = value;
        Buf32::from(buf)
    }

    fn create_test_operator_pubkeys(signing: u8, wallet: u8) -> OperatorPubkeys {
        OperatorPubkeys::new(create_test_buf32(signing), create_test_buf32(wallet))
    }

    #[test]
    fn test_operator_entry_getters() {
        let signing_pk = create_test_buf32(1);
        let wallet_pk = create_test_buf32(2);

        let entry = OperatorEntry {
            idx: 5,
            signing_pk,
            wallet_pk,
        };

        assert_eq!(entry.idx(), 5);
        assert_eq!(entry.signing_pk(), &signing_pk);
        assert_eq!(entry.wallet_pk(), &wallet_pk);
    }

    #[test]
    fn test_operator_table_new_empty() {
        let table = OperatorTable::new_empty();

        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
        assert_eq!(table.operators().len(), 0);
        assert_eq!(table.next_idx, 0);
    }

    #[test]
    fn test_operator_table_from_operator_list() {
        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
            create_test_operator_pubkeys(5, 6),
        ];

        let table = OperatorTable::from_operator_list(&operators);

        assert_eq!(table.len(), 3);
        assert!(!table.is_empty());
        assert_eq!(table.next_idx, 3);

        // Check first operator
        let op0 = table.get_operator(0).unwrap();
        assert_eq!(op0.idx(), 0);
        assert_eq!(op0.signing_pk(), &create_test_buf32(1));
        assert_eq!(op0.wallet_pk(), &create_test_buf32(2));

        // Check second operator
        let op1 = table.get_operator(1).unwrap();
        assert_eq!(op1.idx(), 1);
        assert_eq!(op1.signing_pk(), &create_test_buf32(3));
        assert_eq!(op1.wallet_pk(), &create_test_buf32(4));
    }

    #[test]
    fn test_operator_table_insert() {
        let mut table = OperatorTable::new_empty();

        let signing_pk1 = create_test_buf32(10);
        let wallet_pk1 = create_test_buf32(20);
        table.insert(signing_pk1, wallet_pk1);

        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx, 1);

        let op = table.get_operator(0).unwrap();
        assert_eq!(op.idx(), 0);
        assert_eq!(op.signing_pk(), &signing_pk1);
        assert_eq!(op.wallet_pk(), &wallet_pk1);

        // Insert second operator
        let signing_pk2 = create_test_buf32(30);
        let wallet_pk2 = create_test_buf32(40);
        table.insert(signing_pk2, wallet_pk2);

        assert_eq!(table.len(), 2);
        assert_eq!(table.next_idx, 2);

        let op2 = table.get_operator(1).unwrap();
        assert_eq!(op2.idx(), 1);
        assert_eq!(op2.signing_pk(), &signing_pk2);
        assert_eq!(op2.wallet_pk(), &wallet_pk2);
    }

    #[test]
    fn test_operator_table_get_operator() {
        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
        ];
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
        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
        ];
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
        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
            create_test_operator_pubkeys(5, 6),
        ];
        let table = OperatorTable::from_operator_list(&operators);

        let indices: Vec<_> = table.indices().collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_operator_key_provider() {
        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
        ];
        let table = OperatorTable::from_operator_list(&operators);

        // Test existing operator
        let signing_pk = table.get_operator_signing_pk(0).unwrap();
        assert_eq!(signing_pk, create_test_buf32(2)); // Returns wallet_pk

        let signing_pk2 = table.get_operator_signing_pk(1).unwrap();
        assert_eq!(signing_pk2, create_test_buf32(4)); // Returns wallet_pk

        // Test non-existing operator
        assert!(table.get_operator_signing_pk(2).is_none());
    }

    #[test]
    fn test_operator_table_sanity_check() {
        let table = OperatorTable::new_empty();
        table.sanity_check(); // Should not panic on empty table

        let operators = vec![
            create_test_operator_pubkeys(1, 2),
            create_test_operator_pubkeys(3, 4),
        ];
        let table = OperatorTable::from_operator_list(&operators);
        table.sanity_check(); // Should not panic on valid table
    }
}

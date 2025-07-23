//! Operator-related types and tables.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::OperatorIdx,
    buf::Buf32,
    operator::{OperatorKeyProvider, OperatorPubkeys},
};

/// Entry for an operator.
///
/// Each operator has:
///
/// * an `idx` which is used to identify operators uniquely.
/// * a `signing_pk` which is a [`Buf32`] key used to sign messages sent among each other.
/// * a `wallet_pk` which is a [`Buf32`] [`XOnlyPublickey`](bitcoin::secp256k1::XOnlyPublicKey) used
///   to sign bridge transactions.
///
/// # Note
///
/// The separation between the two keys is so that we can use a different signing mechanism for
/// signing messages in the future. For the present, only the `wallet_pk` is used.
///
/// Also note that the `wallet_pk` corresponds to a [`PublicKey`](bitcoin::secp256k1::PublicKey)
/// with an even parity as per [BIP 340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki#design).
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
    pub fn idx(&self) -> OperatorIdx {
        self.idx
    }

    /// Get pubkey used to verify signed messages from the operator.
    pub fn signing_pk(&self) -> &Buf32 {
        &self.signing_pk
    }

    /// Get wallet pubkey used to compute MuSig2 pubkey from a set of operators.
    pub fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct OperatorTable {
    /// Next unassigned operator index.
    next_idx: OperatorIdx,

    /// Operator table.
    ///
    /// MUST be sorted by `idx`.
    operators: Vec<OperatorEntry>,
}

impl OperatorTable {
    pub fn new_empty() -> Self {
        Self {
            next_idx: 0,
            operators: Vec::new(),
        }
    }

    /// Constructs an operator table from a list of operator indexes.
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

    /// Sanity checks the operator table for sensibility.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.operators.is_sorted_by_key(|e| e.idx) {
            panic!("bridge_state: operators list not sorted");
        }

        if let Some(e) = self.operators.last() && self.next_idx <= e.idx {
            panic!("bridge_state: operators next_idx before last entry");
        }
    }

    /// Returns the number of operator entries.
    pub fn len(&self) -> u32 {
        self.operators.len() as u32
    }

    /// Returns if the operator table is empty.  This is practically probably
    /// never going to be true.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    pub fn operators(&self) -> &[OperatorEntry] {
        &self.operators
    }

    /// Inserts a new operator entry.
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

    /// Gets an operator from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_operator(&self, idx: u32) -> Option<&OperatorEntry> {
        self.operators
            .binary_search_by_key(&idx, |e| e.idx)
            .ok()
            .map(|i| &self.operators[i])
    }

    /// Gets a operator entry by its internal position, *ignoring* the indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&OperatorEntry> {
        self.operators.get(pos as usize)
    }

    /// Get all the operator's index
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
    use super::*;
    use strata_primitives::operator::OperatorPubkeys;

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
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
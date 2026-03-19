use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::{AccountId, MessageEntry, TxEffects};
use strata_identifiers::{Buf32, OLTxId, Slot};
use tree_hash::{Sha256Hasher, TreeHash};

use crate::ssz_generated::ssz::{proofs::*, transaction::*};

impl OLTransaction {
    pub fn new(data: OLTransactionData, proofs: TxProofs) -> Self {
        Self { data, proofs }
    }

    pub fn data(&self) -> &OLTransactionData {
        &self.data
    }

    pub fn proofs(&self) -> &TxProofs {
        &self.proofs
    }

    pub fn constraints(&self) -> &TxConstraints {
        &self.data.constraints
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.data.payload
    }

    pub fn target(&self) -> Option<AccountId> {
        self.payload().target()
    }

    pub fn type_id(&self) -> TxTypeId {
        self.payload().type_id()
    }

    pub fn compute_txid(&self) -> OLTxId {
        self.data().compute_txid()
    }
}

impl TransactionPayload {
    pub fn target(&self) -> Option<AccountId> {
        match self {
            TransactionPayload::GenericAccountMessage(msg) => Some(msg.target),
            TransactionPayload::SnarkAccountUpdate(update) => Some(update.target),
        }
    }

    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayload::GenericAccountMessage(_) => TxTypeId::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate(_) => TxTypeId::SnarkAccountUpdate,
        }
    }
}

impl TxConstraints {
    pub fn new(min_slot: Option<Slot>, max_slot: Option<Slot>) -> Self {
        Self {
            min_slot: min_slot.into(),
            max_slot: max_slot.into(),
        }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        match &self.min_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot.into();
    }

    pub fn max_slot(&self) -> Option<Slot> {
        match &self.max_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot.into();
    }
}

/// Type ID to indicate transaction types.
#[repr(u16)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, IntEnum)]
pub enum TxTypeId {
    /// Transactions that are messages being sent to other accounts.
    GenericAccountMessage = 0,

    /// Transactions that are snark account updates.
    SnarkAccountUpdate = 1,
}

impl fmt::Display for TxTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TxTypeId::GenericAccountMessage => "generic-account-message",
            TxTypeId::SnarkAccountUpdate => "snark-account-update",
        };
        f.write_str(s)
    }
}

impl GamTxPayload {
    pub fn new(target: AccountId) -> Result<Self, &'static str> {
        Ok(Self { target })
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }
}

impl SauTxPayload {
    /// Creates a new snark account update transaction payload.
    pub fn new(target: AccountId, operation_data: SauTxOperationData) -> Self {
        Self {
            target,
            operation_data,
        }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn operation(&self) -> &SauTxOperationData {
        &self.operation_data
    }
}

impl SauTxOperationData {
    /// Creates a new operation data.
    pub fn new(
        update_data: SauTxUpdateData,
        messages: Vec<MessageEntry>,
        ledger_refs: SauTxLedgerRefs,
    ) -> Self {
        Self {
            update_data,
            messages: messages.into(),
            ledger_refs,
        }
    }

    pub fn update(&self) -> &SauTxUpdateData {
        &self.update_data
    }

    pub fn messages_iter(&self) -> impl Iterator<Item = &MessageEntry> {
        self.messages.iter()
    }

    pub fn ledger_refs(&self) -> &SauTxLedgerRefs {
        &self.ledger_refs
    }
}

impl SauTxLedgerRefs {
    /// Creates empty ledger refs.
    pub fn new_empty() -> Self {
        Self {
            asm_history_proofs: ssz_types::Optional::None,
        }
    }

    /// Creates ledger refs with the given claim list.
    pub fn new_with_claims(claims: ClaimList) -> Self {
        Self {
            asm_history_proofs: ssz_types::Optional::Some(claims),
        }
    }

    pub fn asm_history_proofs(&self) -> Option<&ClaimList> {
        match self.asm_history_proofs.as_ref() {
            ssz_types::Optional::None => None,
            ssz_types::Optional::Some(l) => Some(l),
        }
    }
}

impl SauTxUpdateData {
    /// Creates a new update data.
    pub fn new(seq_no: u64, proof_state: SauTxProofState, extra_data: Vec<u8>) -> Self {
        Self {
            seq_no,
            proof_state,
            extra_data: extra_data.into(),
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn proof_state(&self) -> &SauTxProofState {
        &self.proof_state
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

impl SauTxProofState {
    /// Creates a new proof state.
    pub fn new(new_next_msg_idx: u64, inner_state_root: Buf32) -> Self {
        Self {
            new_next_msg_idx,
            inner_state_root: inner_state_root.0.into(),
        }
    }

    pub fn new_next_msg_idx(&self) -> u64 {
        self.new_next_msg_idx
    }

    pub fn inner_state_root(&self) -> Buf32 {
        self.inner_state_root.0.into()
    }
}

impl OLTransactionData {
    /// Creates a new transaction data with the given payload and effects, and default constraints.
    pub fn new(payload: TransactionPayload, effects: TxEffects) -> Self {
        Self {
            payload,
            constraints: TxConstraints::default(),
            effects,
        }
    }

    /// Sets the constraints on this transaction data, consuming and returning self.
    pub fn with_constraints(mut self, constraints: TxConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn constraints(&self) -> &TxConstraints {
        &self.constraints
    }

    pub fn effects(&self) -> &TxEffects {
        &self.effects
    }

    /// Computes the txid.
    pub fn compute_txid(&self) -> OLTxId {
        let txid_raw = <Self as TreeHash<Sha256Hasher>>::tree_hash_root(self);
        OLTxId::from(Buf32::from(txid_raw.0))
    }
}

impl TxProofs {
    /// Creates an empty TxProofs with no satisfiers or accumulator proofs.
    pub fn new_empty() -> Self {
        Self {
            predicate_satisfiers: ssz_types::Optional::None,
            accumulator_proofs: ssz_types::Optional::None,
        }
    }

    /// Creates TxProofs with the given satisfiers and accumulator proofs.
    pub fn new(
        predicate_satisfiers: Option<ProofSatisfierList>,
        accumulator_proofs: Option<RawMerkleProofList>,
    ) -> Self {
        Self {
            predicate_satisfiers: predicate_satisfiers.into(),
            accumulator_proofs: accumulator_proofs.into(),
        }
    }

    pub fn predicate_satisfiers(&self) -> Option<&ProofSatisfierList> {
        match &self.predicate_satisfiers {
            ssz_types::Optional::Some(s) => Some(s),
            ssz_types::Optional::None => None,
        }
    }

    pub fn accumulator_proofs(&self) -> Option<&RawMerkleProofList> {
        match &self.accumulator_proofs {
            ssz_types::Optional::Some(p) => Some(p),
            ssz_types::Optional::None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        test_utils::{
            gam_tx_payload_strategy, ol_transaction_strategy, transaction_attachment_strategy,
            transaction_payload_strategy,
        },
        *,
    };

    mod tx_constraints {
        use super::*;

        ssz_proptest!(TxConstraints, transaction_attachment_strategy());

        #[test]
        fn test_none_values() {
            let attachment = TxConstraints {
                min_slot: ssz_types::Optional::None,
                max_slot: ssz_types::Optional::None,
            };
            let encoded = attachment.as_ssz_bytes();
            let decoded = TxConstraints::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(attachment, decoded);
        }
    }

    mod gam_tx_payload {
        use super::*;

        ssz_proptest!(GamTxPayload, gam_tx_payload_strategy());

        #[test]
        fn test_roundtrip() {
            let msg = GamTxPayload {
                target: AccountId::from([0u8; 32]),
            };
            let encoded = msg.as_ssz_bytes();
            let decoded = GamTxPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    mod transaction_payload {
        use super::*;

        ssz_proptest!(TransactionPayload, transaction_payload_strategy());

        #[test]
        fn test_gam_tx_payload_variant() {
            let payload = TransactionPayload::GenericAccountMessage(GamTxPayload {
                target: AccountId::from([0u8; 32]),
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }

        #[test]
        fn test_snark_account_update_tx_payload_variant() {
            let payload = TransactionPayload::SnarkAccountUpdate(SauTxPayload {
                target: AccountId::from([0u8; 32]),
                operation_data: SauTxOperationData {
                    update_data: SauTxUpdateData {
                        seq_no: 1,
                        proof_state: SauTxProofState {
                            new_next_msg_idx: 0,
                            inner_state_root: [0u8; 32].into(),
                        },
                        extra_data: vec![].into(),
                    },
                    messages: vec![].into(),
                    ledger_refs: SauTxLedgerRefs {
                        asm_history_proofs: ssz_types::Optional::None,
                    },
                },
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    mod ol_transaction {
        use strata_acct_types::TxEffects;

        use super::*;

        ssz_proptest!(OLTransaction, ol_transaction_strategy());

        #[test]
        fn test_generic_message() {
            let tx = OLTransaction {
                data: OLTransactionData {
                    payload: TransactionPayload::GenericAccountMessage(GamTxPayload {
                        target: AccountId::from([0u8; 32]),
                    }),
                    constraints: TxConstraints::default(),
                    effects: TxEffects::default(),
                },
                proofs: TxProofs {
                    predicate_satisfiers: ssz_types::Optional::None,
                    accumulator_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }

        #[test]
        fn test_snark_account_update() {
            let tx = OLTransaction {
                data: OLTransactionData {
                    payload: TransactionPayload::SnarkAccountUpdate(SauTxPayload {
                        target: AccountId::from([1u8; 32]),
                        operation_data: SauTxOperationData {
                            update_data: SauTxUpdateData {
                                seq_no: 42,
                                proof_state: SauTxProofState {
                                    new_next_msg_idx: 10,
                                    inner_state_root: [5u8; 32].into(),
                                },
                                extra_data: vec![].into(),
                            },
                            messages: vec![].into(),
                            ledger_refs: SauTxLedgerRefs {
                                asm_history_proofs: ssz_types::Optional::None,
                            },
                        },
                    }),
                    constraints: TxConstraints {
                        min_slot: ssz_types::Optional::Some(100),
                        max_slot: ssz_types::Optional::Some(200),
                    },
                    effects: TxEffects::default(),
                },
                proofs: TxProofs {
                    predicate_satisfiers: ssz_types::Optional::None,
                    accumulator_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }
    }
}

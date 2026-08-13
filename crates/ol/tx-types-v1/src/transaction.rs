use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::{
    AccountId, BitcoinAmount, MessageEntry, MsgPayload, MsgPayloadData, MsgPayloadError,
    SentMessage, TxEffects,
};
use strata_identifiers::{Buf32, OLTxId, Slot};
use strata_ol_logs::SnarkAccountUpdateLogData;
use strata_predicate::PredicateKey;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::ssz_generated::ssz::{proofs::*, transaction::*};

impl OLTransactionV1 {
    pub fn new(data: OLTransactionDataV1, proofs: TxProofsV1) -> Self {
        Self { data, proofs }
    }

    pub fn data(&self) -> &OLTransactionDataV1 {
        &self.data
    }

    pub fn proofs(&self) -> &TxProofsV1 {
        &self.proofs
    }

    pub fn constraints(&self) -> &TxConstraintsV1 {
        &self.data.constraints
    }

    pub fn payload(&self) -> &TransactionPayloadV1 {
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

    /// Returns a new transaction with only accumulator proofs updated.
    pub fn with_accumulator_proofs(
        mut self,
        accumulator_proofs: Option<RawMerkleProofListV1>,
    ) -> Self {
        self.proofs = self.proofs.with_accumulator_proofs(accumulator_proofs);
        self
    }
}

impl TransactionPayloadV1 {
    pub fn target(&self) -> Option<AccountId> {
        match self {
            TransactionPayloadV1::GenericAccountMessage(msg) => Some(msg.target),
            TransactionPayloadV1::SnarkAccountUpdate(update) => Some(update.target),
        }
    }

    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayloadV1::GenericAccountMessage(_) => TxTypeId::GenericAccountMessage,
            TransactionPayloadV1::SnarkAccountUpdate(_) => TxTypeId::SnarkAccountUpdate,
        }
    }
}

impl TxConstraintsV1 {
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

impl GamTxPayloadV1 {
    pub fn new(target: AccountId) -> Result<Self, &'static str> {
        Ok(Self { target })
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }
}

impl SauTxPayloadV1 {
    /// Creates a new snark account update transaction payload.
    pub fn new(target: AccountId, operation_data: SauTxOperationDataV1) -> Self {
        Self {
            target,
            operation_data,
        }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn operation(&self) -> &SauTxOperationDataV1 {
        &self.operation_data
    }
}

impl SauTxOperationDataV1 {
    /// Creates a new operation data.
    pub fn new(
        update_data: SauTxUpdateDataV1,
        messages: Vec<MessageEntry>,
        ledger_refs: SauTxLedgerRefsV1,
    ) -> Self {
        Self {
            update_data,
            messages: messages
                .try_into()
                .expect("messages must fit within SSZ max length"),
            ledger_refs,
        }
    }

    pub fn update(&self) -> &SauTxUpdateDataV1 {
        &self.update_data
    }

    pub fn messages_iter(&self) -> impl Iterator<Item = &MessageEntry> {
        self.messages.iter()
    }

    pub fn ledger_refs(&self) -> &SauTxLedgerRefsV1 {
        &self.ledger_refs
    }
}

impl SauTxLedgerRefsV1 {
    /// Creates empty ledger refs.
    pub fn new_empty() -> Self {
        Self {
            l1_block_ref_claims: ssz_types::Optional::None,
        }
    }

    /// Creates ledger refs with the given claims.
    pub fn new_with_claims(claims: ClaimListV1) -> Self {
        Self {
            l1_block_ref_claims: ssz_types::Optional::Some(claims),
        }
    }

    pub fn set_l1_block_ref_claims(&mut self, claims: ClaimListV1) {
        self.l1_block_ref_claims = ssz_types::Optional::Some(claims);
    }

    pub fn l1_block_ref_claims(&self) -> Option<&ClaimListV1> {
        match self.l1_block_ref_claims.as_ref() {
            ssz_types::Optional::None => None,
            ssz_types::Optional::Some(l) => Some(l),
        }
    }
}

impl SauTxNewPredicateV1 {
    /// Creates an empty declaration (no predicate rotation).
    pub fn new_empty() -> Self {
        Self {
            predicate: ssz_types::Optional::None,
        }
    }

    /// Creates a declaration rotating to the given predicate key.
    pub fn new_with_key(key: PredicateKey) -> Self {
        Self {
            predicate: ssz_types::Optional::Some(key),
        }
    }

    pub fn predicate(&self) -> Option<&PredicateKey> {
        match &self.predicate {
            ssz_types::Optional::Some(key) => Some(key),
            ssz_types::Optional::None => None,
        }
    }
}

impl From<Option<PredicateKey>> for SauTxNewPredicateV1 {
    fn from(key: Option<PredicateKey>) -> Self {
        Self {
            predicate: key.into(),
        }
    }
}

impl SauTxUpdateDataV1 {
    /// Creates a new update data.
    pub fn new(
        seq_no: u64,
        proof_state: SauTxProofStateV1,
        extra_data: Vec<u8>,
        new_predicate: Option<PredicateKey>,
    ) -> Self {
        Self {
            seq_no,
            proof_state,
            extra_data: extra_data
                .try_into()
                .expect("extra data must fit within SSZ max length"),
            new_predicate: new_predicate.into(),
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn proof_state(&self) -> &SauTxProofStateV1 {
        &self.proof_state
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// Returns the new update predicate key this update rotates to, if any.
    pub fn new_predicate(&self) -> Option<&PredicateKey> {
        self.new_predicate.predicate()
    }

    /// Builds the [`SnarkAccountUpdateLogData`] emitted for this update.
    ///
    /// Returns `None` if the update's extra data exceeds the log payload bound. That bound
    /// matches the SSZ `SAU_MAX_EXTRA_DATA_BYTES` cap, so a well-formed update always fits.
    pub fn get_log_data(&self) -> Option<SnarkAccountUpdateLogData> {
        SnarkAccountUpdateLogData::new(
            self.proof_state().new_next_msg_idx(),
            self.extra_data().to_vec(),
        )
    }
}

impl SauTxProofStateV1 {
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

impl OLTransactionDataV1 {
    /// Creates a new transaction data with the given payload and effects, and default constraints.
    pub fn new(payload: TransactionPayloadV1, effects: TxEffects) -> Self {
        Self {
            payload,
            constraints: TxConstraintsV1::default(),
            effects,
        }
    }

    /// Creates a GAM transaction data targeting the given account with a zero-value message
    /// containing the provided payload data.
    pub fn new_gam(dest: AccountId, data: MsgPayloadData) -> Self {
        let payload = TransactionPayloadV1::GenericAccountMessage(GamTxPayloadV1 { target: dest });
        let mut effects = TxEffects::default();
        effects.add_message(SentMessage::new(
            dest,
            MsgPayload::new(BitcoinAmount::default(), data),
        ));
        Self {
            payload,
            constraints: TxConstraintsV1::default(),
            effects,
        }
    }

    /// Creates GAM transaction data from raw message payload bytes.
    pub fn from_gam_bytes(dest: AccountId, data: Vec<u8>) -> Result<Self, MsgPayloadError> {
        let msg_payload = MsgPayload::from_bytes_valueless(data)?;
        Ok(Self::new_gam(dest, msg_payload.data))
    }

    /// Sets the constraints on this transaction data, consuming and returning self.
    pub fn with_constraints(mut self, constraints: TxConstraintsV1) -> Self {
        self.constraints = constraints;
        self
    }

    pub fn payload(&self) -> &TransactionPayloadV1 {
        &self.payload
    }

    pub fn constraints(&self) -> &TxConstraintsV1 {
        &self.constraints
    }

    pub fn effects(&self) -> &TxEffects {
        &self.effects
    }

    /// Computes the txid.
    pub fn compute_txid(&self) -> OLTxId {
        let txid_raw = <Self as TreeHash>::tree_hash_root::<Sha256Hasher>(self);
        OLTxId::from(Buf32::from(txid_raw.0))
    }
}

impl TxProofsV1 {
    /// Creates an empty TxProofsV1 with no satisfiers or accumulator proofs.
    pub fn new_empty() -> Self {
        Self {
            predicate_satisfiers: ssz_types::Optional::None,
            accumulator_proofs: ssz_types::Optional::None,
        }
    }

    /// Creates TxProofsV1 with the given satisfiers and accumulator proofs.
    pub fn new(
        predicate_satisfiers: Option<ProofSatisfierListV1>,
        accumulator_proofs: Option<RawMerkleProofListV1>,
    ) -> Self {
        Self {
            predicate_satisfiers: predicate_satisfiers.into(),
            accumulator_proofs: accumulator_proofs.into(),
        }
    }

    pub fn predicate_satisfiers(&self) -> Option<&ProofSatisfierListV1> {
        match &self.predicate_satisfiers {
            ssz_types::Optional::Some(s) => Some(s),
            ssz_types::Optional::None => None,
        }
    }

    pub fn accumulator_proofs(&self) -> Option<&RawMerkleProofListV1> {
        match &self.accumulator_proofs {
            ssz_types::Optional::Some(p) => Some(p),
            ssz_types::Optional::None => None,
        }
    }

    /// Returns a new [`TxProofsV1`] with only accumulator proofs updated.
    pub fn with_accumulator_proofs(
        mut self,
        accumulator_proofs: Option<RawMerkleProofListV1>,
    ) -> Self {
        self.accumulator_proofs = accumulator_proofs.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_predicate::PredicateKey;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        test_utils::{
            gam_tx_payload_strategy, ol_transaction_strategy, transaction_payload_strategy,
            tx_constraints_strategy,
        },
        *,
    };

    mod tx_constraints {
        use super::*;

        ssz_proptest!(TxConstraintsV1, tx_constraints_strategy());

        #[test]
        fn test_none_values() {
            let attachment = TxConstraintsV1 {
                min_slot: ssz_types::Optional::None,
                max_slot: ssz_types::Optional::None,
            };
            let encoded = attachment.as_ssz_bytes();
            let decoded = TxConstraintsV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(attachment, decoded);
        }
    }

    mod gam_tx_payload {
        use super::*;

        ssz_proptest!(GamTxPayloadV1, gam_tx_payload_strategy());

        #[test]
        fn test_roundtrip() {
            let msg = GamTxPayloadV1 {
                target: AccountId::from([0u8; 32]),
            };
            let encoded = msg.as_ssz_bytes();
            let decoded = GamTxPayloadV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    mod transaction_payload {
        use super::*;

        ssz_proptest!(TransactionPayloadV1, transaction_payload_strategy());

        #[test]
        fn test_gam_tx_payload_variant() {
            let payload = TransactionPayloadV1::GenericAccountMessage(GamTxPayloadV1 {
                target: AccountId::from([0u8; 32]),
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayloadV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }

        #[test]
        fn test_snark_account_update_tx_payload_variant() {
            let payload = TransactionPayloadV1::SnarkAccountUpdate(SauTxPayloadV1 {
                target: AccountId::from([0u8; 32]),
                operation_data: SauTxOperationDataV1 {
                    update_data: SauTxUpdateDataV1 {
                        seq_no: 1,
                        proof_state: SauTxProofStateV1 {
                            new_next_msg_idx: 0,
                            inner_state_root: [0u8; 32].into(),
                        },
                        extra_data: Vec::new()
                            .try_into()
                            .expect("extra data must fit within SSZ max length"),
                        new_predicate: SauTxNewPredicateV1::new_empty(),
                    },
                    messages: Vec::new()
                        .try_into()
                        .expect("messages must fit within SSZ max length"),
                    ledger_refs: SauTxLedgerRefsV1 {
                        l1_block_ref_claims: ssz_types::Optional::None,
                    },
                },
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayloadV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    mod ol_transaction {
        use strata_acct_types::TxEffects;

        use super::*;

        ssz_proptest!(OLTransactionV1, ol_transaction_strategy());

        #[test]
        fn test_generic_message() {
            let tx = OLTransactionV1 {
                data: OLTransactionDataV1 {
                    payload: TransactionPayloadV1::GenericAccountMessage(GamTxPayloadV1 {
                        target: AccountId::from([0u8; 32]),
                    }),
                    constraints: TxConstraintsV1::default(),
                    effects: TxEffects::default(),
                },
                proofs: TxProofsV1 {
                    predicate_satisfiers: ssz_types::Optional::None,
                    accumulator_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransactionV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }

        #[test]
        fn test_snark_account_update() {
            let tx = OLTransactionV1 {
                data: OLTransactionDataV1 {
                    payload: TransactionPayloadV1::SnarkAccountUpdate(SauTxPayloadV1 {
                        target: AccountId::from([1u8; 32]),
                        operation_data: SauTxOperationDataV1 {
                            update_data: SauTxUpdateDataV1 {
                                seq_no: 42,
                                proof_state: SauTxProofStateV1 {
                                    new_next_msg_idx: 10,
                                    inner_state_root: [5u8; 32].into(),
                                },
                                extra_data: Vec::new()
                                    .try_into()
                                    .expect("extra data must fit within SSZ max length"),
                                new_predicate: SauTxNewPredicateV1::new_with_key(
                                    PredicateKey::always_accept(),
                                ),
                            },
                            messages: Vec::new()
                                .try_into()
                                .expect("messages must fit within SSZ max length"),
                            ledger_refs: SauTxLedgerRefsV1 {
                                l1_block_ref_claims: ssz_types::Optional::None,
                            },
                        },
                    }),
                    constraints: TxConstraintsV1 {
                        min_slot: ssz_types::Optional::Some(100),
                        max_slot: ssz_types::Optional::Some(200),
                    },
                    effects: TxEffects::default(),
                },
                proofs: TxProofsV1 {
                    predicate_satisfiers: ssz_types::Optional::None,
                    accumulator_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransactionV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }
    }
}

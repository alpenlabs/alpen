use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::{
    AccountId, AccumulatorClaim, BitcoinAmount, MessageEntry, MsgPayload, MsgPayloadData,
    MsgPayloadError, RawMerkleProof, SentMessage, TxEffects,
};
use strata_identifiers::{Buf32, OLTxId, Slot};
use strata_ol_logs::SnarkAccountUpdateLogData;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::{
    chain_abstraction::*,
    ssz_generated::ssz::{proofs::*, transaction::*},
};

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
        ITransaction::compute_txid(&self)
    }

    /// Returns a new transaction with only accumulator proofs updated.
    pub fn with_accumulator_proofs(
        mut self,
        accumulator_proofs: Option<RawMerkleProofList>,
    ) -> Self {
        self.proofs = self.proofs.with_accumulator_proofs(accumulator_proofs);
        self
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

impl<'tx> ITransaction for &'tx OLTransaction {
    type Constraints = &'tx TxConstraints;
    type Proofs = &'tx TxProofs;
    type Gam = &'tx GamTxPayload;
    type Sau = &'tx SauTxPayload;

    fn compute_txid(&self) -> OLTxId {
        self.data().compute_txid()
    }

    fn tydata(&self) -> TxTyData<Self> {
        // Copy out the outer reference so the borrows live for `'tx`, not just the
        // duration of `&self`.
        let tx: &'tx OLTransaction = *self;
        match tx.payload() {
            TransactionPayload::GenericAccountMessage(pl) => TxTyData::GenericAcctMessage(pl),
            TransactionPayload::SnarkAccountUpdate(pl) => TxTyData::SnarkAcctUpdate(pl),
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

    #[deprecated(note = "use ITxConstraints trait")]
    pub fn min_slot(&self) -> Option<Slot> {
        ITxConstraints::min_slot(&self)
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot.into();
    }

    #[deprecated(note = "use ITxConstraints trait")]
    pub fn max_slot(&self) -> Option<Slot> {
        ITxConstraints::max_slot(&self)
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot.into();
    }
}

impl<'tx> ITxConstraints for &'tx TxConstraints {
    fn min_slot(&self) -> Option<Slot> {
        match &self.min_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
    }

    fn max_slot(&self) -> Option<Slot> {
        match &self.max_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
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

impl<'tx> ITargetTx for &'tx GamTxPayload {
    fn target(&self) -> AccountId {
        self.target
    }
}

impl<'tx> IGamTransaction for &'tx GamTxPayload {}

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

impl<'tx> ITargetTx for &'tx SauTxPayload {
    fn target(&self) -> AccountId {
        self.target
    }
}

impl<'tx> ISauTransaction for &'tx SauTxPayload {
    type Operation = &'tx SauTxOperationData;

    fn operation(&self) -> Self::Operation {
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
            messages: messages
                .try_into()
                .expect("messages must fit within SSZ max length"),
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

impl<'tx> ISauOperationData for &'tx SauTxOperationData {
    type Data = &'tx SauTxUpdateData;
    type Message = &'tx MessageEntry;
    type LedgerRefs = &'tx SauTxLedgerRefs;

    fn update_data(&self) -> Self::Data {
        &self.update_data
    }

    fn iter_messages(&self) -> impl Iterator<Item = Self::Message> {
        self.messages.iter()
    }

    fn ledger_refs(&self) -> Self::LedgerRefs {
        &self.ledger_refs
    }
}

impl SauTxLedgerRefs {
    /// Creates empty ledger refs.
    pub fn new_empty() -> Self {
        Self {
            l1_block_ref_claims: ssz_types::Optional::None,
        }
    }

    /// Creates ledger refs with the given claims.
    pub fn new_with_claims(claims: ClaimList) -> Self {
        Self {
            l1_block_ref_claims: ssz_types::Optional::Some(claims),
        }
    }

    pub fn set_l1_block_ref_claims(&mut self, claims: ClaimList) {
        self.l1_block_ref_claims = ssz_types::Optional::Some(claims);
    }

    pub fn l1_block_ref_claims(&self) -> Option<&ClaimList> {
        match self.l1_block_ref_claims.as_ref() {
            ssz_types::Optional::None => None,
            ssz_types::Optional::Some(l) => Some(l),
        }
    }
}

impl<'tx> ISauLedgerRefs for &'tx SauTxLedgerRefs {
    fn num_l1_block_ref_claims(&self) -> usize {
        self.l1_block_ref_claims()
            .map(|l| l.claims().len())
            .unwrap_or_default()
    }

    fn get_l1_block_ref_claim(&self, idx: usize) -> Option<AccumulatorClaim> {
        self.l1_block_ref_claims()
            .and_then(|l| l.claims().get(idx))
            .cloned()
    }
}

impl SauTxUpdateData {
    /// Creates a new update data.
    pub fn new(seq_no: u64, proof_state: SauTxProofState, extra_data: Vec<u8>) -> Self {
        Self {
            seq_no,
            proof_state,
            extra_data: extra_data
                .try_into()
                .expect("extra data must fit within SSZ max length"),
        }
    }

    #[deprecated(note = "use ISauUpdateData trait")]
    pub fn seq_no(&self) -> u64 {
        ISauUpdateData::seq_no(&self)
    }

    pub fn proof_state(&self) -> &SauTxProofState {
        &self.proof_state
    }

    #[deprecated(note = "use ISauUpdateData trait")]
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// Builds the [`SnarkAccountUpdateLogData`] emitted for this update.
    ///
    /// Returns `None` if the update's extra data exceeds the log payload bound. That bound
    /// matches the SSZ `SAU_MAX_EXTRA_DATA_BYTES` cap, so a well-formed update always fits.
    pub fn get_log_data(&self) -> Option<SnarkAccountUpdateLogData> {
        SnarkAccountUpdateLogData::new(
            self.proof_state().new_next_msg_idx(),
            ISauUpdateData::extra_data(&self).to_vec(),
        )
    }
}

impl<'tx> ISauUpdateData for &'tx SauTxUpdateData {
    fn seq_no(&self) -> u64 {
        self.seq_no
    }

    fn new_next_msg_idx(&self) -> u64 {
        self.proof_state().new_next_msg_idx()
    }

    fn new_inner_state_root(&self) -> Buf32 {
        self.proof_state().inner_state_root().into()
    }

    fn extra_data(&self) -> &[u8] {
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

    /// Creates a GAM transaction data targeting the given account with a zero-value message
    /// containing the provided payload data.
    pub fn new_gam(dest: AccountId, data: MsgPayloadData) -> Self {
        let payload = TransactionPayload::GenericAccountMessage(GamTxPayload { target: dest });
        let mut effects = TxEffects::default();
        effects.add_message(SentMessage::new(
            dest,
            MsgPayload::new(BitcoinAmount::zero(), data),
        ));
        Self {
            payload,
            constraints: TxConstraints::default(),
            effects,
        }
    }

    /// Creates GAM transaction data from raw message payload bytes.
    pub fn from_gam_bytes(dest: AccountId, data: Vec<u8>) -> Result<Self, MsgPayloadError> {
        let msg_payload = MsgPayload::from_bytes_valueless(data)?;
        Ok(Self::new_gam(dest, msg_payload.data))
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
        let txid_raw = <Self as TreeHash>::tree_hash_root::<Sha256Hasher>(self);
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

    /// Returns a new [`TxProofs`] with only accumulator proofs updated.
    pub fn with_accumulator_proofs(
        mut self,
        accumulator_proofs: Option<RawMerkleProofList>,
    ) -> Self {
        self.accumulator_proofs = accumulator_proofs.into();
        self
    }
}

impl<'tx> ITxProofs for &'tx TxProofs {
    fn num_predicate_satisfiers(&self) -> usize {
        self.predicate_satisfiers()
            .map(|l| l.proofs().len())
            .unwrap_or_default()
    }

    fn get_predicate_satisfier(&self, idx: usize) -> Option<ProofSatisfier> {
        self.predicate_satisfiers()
            .and_then(|l| l.proofs().get(idx))
            .cloned()
    }

    fn num_accumulator_proofs(&self) -> usize {
        self.accumulator_proofs()
            .map(|l| l.proofs().len())
            .unwrap_or_default()
    }

    fn get_accumulator_proof(&self, idx: usize) -> Option<RawMerkleProof> {
        self.accumulator_proofs()
            .and_then(|l| l.proofs().get(idx))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
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

        ssz_proptest!(TxConstraints, tx_constraints_strategy());

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
                        extra_data: Vec::new()
                            .try_into()
                            .expect("extra data must fit within SSZ max length"),
                    },
                    messages: Vec::new()
                        .try_into()
                        .expect("messages must fit within SSZ max length"),
                    ledger_refs: SauTxLedgerRefs {
                        l1_block_ref_claims: ssz_types::Optional::None,
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
                                extra_data: Vec::new()
                                    .try_into()
                                    .expect("extra data must fit within SSZ max length"),
                            },
                            messages: Vec::new()
                                .try_into()
                                .expect("messages must fit within SSZ max length"),
                            ledger_refs: SauTxLedgerRefs {
                                l1_block_ref_claims: ssz_types::Optional::None,
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

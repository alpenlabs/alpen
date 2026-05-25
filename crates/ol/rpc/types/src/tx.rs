//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use ssz::Decode;
use strata_acct_types::{AccountId, SentMessage, SentTransfer, TxEffects};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::{
    ClaimList, OLTransaction, OLTransactionData, ProofSatisfierList, SauTxLedgerRefs,
    SauTxOperationData, SauTxPayload, SauTxProofState, SauTxUpdateData, TransactionPayload,
    TxConstraints, TxProofs,
};
use strata_primitives::{HexBytes, HexBytes32};
use strata_snark_acct_types::{SnarkAccountUpdate, UpdateOperationData};

use crate::RpcSnarkAccountUpdate;

/// OL transaction for submission (excludes accumulator proofs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLTransaction {
    /// The payload.
    payload: RpcTransactionPayload,

    /// The constraints.
    constraints: RpcTxConstraints,
}

impl RpcOLTransaction {
    /// Creates a new [`RpcOLTransaction`].
    pub fn new(payload: RpcTransactionPayload, constraints: RpcTxConstraints) -> Self {
        Self {
            payload,
            constraints,
        }
    }

    pub fn new_payload(payload: RpcTransactionPayload) -> Self {
        Self::new(payload, RpcTxConstraints::default())
    }

    pub fn new_snark_acct_update(update: RpcSnarkAccountUpdate) -> Self {
        Self::new_payload(RpcTransactionPayload::SnarkAccountUpdate(update))
    }

    /// Returns the payload.
    pub fn payload(&self) -> &RpcTransactionPayload {
        &self.payload
    }

    /// Returns the constraints.
    pub fn constraints(&self) -> &RpcTxConstraints {
        &self.constraints
    }

    /// Sets the constraints.
    pub fn set_constraints(&mut self, constraints: RpcTxConstraints) {
        self.constraints = constraints;
    }
}

/// Transaction payload: generic message or snark account update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcTransactionPayload {
    /// Generic account message.
    GenericAccountMessage(RpcGenericAccountMessage),

    /// Snark account update.
    SnarkAccountUpdate(RpcSnarkAccountUpdate),
}

/// Generic account message payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcGenericAccountMessage {
    /// The target account.
    target: HexBytes32,

    /// The payload.
    payload: HexBytes,
}

impl RpcGenericAccountMessage {
    /// Creates a new [`RpcGenericAccountMessage`].
    pub fn new(target: HexBytes32, payload: HexBytes) -> Self {
        Self { target, payload }
    }

    /// Returns the target account.
    pub fn target(&self) -> &HexBytes32 {
        &self.target
    }

    /// Returns the payload.
    pub fn payload(&self) -> &HexBytes {
        &self.payload
    }
}

/// Transaction constraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcTxConstraints {
    /// Minimum slot.
    min_slot: Option<u64>,

    /// Maximum slot.
    max_slot: Option<u64>,
}

impl RpcTxConstraints {
    /// Creates a new [`RpcTxConstraints`].
    pub fn new(min_slot: Option<u64>, max_slot: Option<u64>) -> Self {
        Self { min_slot, max_slot }
    }

    /// Returns the minimum slot.
    pub fn min_slot(&self) -> Option<u64> {
        self.min_slot
    }

    /// Returns the maximum slot.
    pub fn max_slot(&self) -> Option<u64> {
        self.max_slot
    }
}

impl From<TxConstraints> for RpcTxConstraints {
    fn from(constraints: TxConstraints) -> Self {
        Self {
            min_slot: constraints.min_slot(),
            max_slot: constraints.max_slot(),
        }
    }
}

impl From<RpcTxConstraints> for TxConstraints {
    fn from(rpc: RpcTxConstraints) -> Self {
        TxConstraints::new(rpc.min_slot, rpc.max_slot)
    }
}

/// Error type for transaction conversion.
#[derive(Debug, thiserror::Error)]
pub enum RpcTxConversionError {
    /// Failed to decode update operation data.
    #[error("failed to decode update operation data: {0}")]
    DecodeOperationData(String),

    /// Too many ASM history claims in snark operation.
    #[error("too many ASM history claims in snark operation")]
    TooManyAsmHistoryClaims,

    /// Message payload data exceeds SSZ limits.
    #[error("invalid message payload: {0}")]
    InvalidMessagePayload(#[from] strata_acct_types::MsgPayloadError),
}

/// Decoded view of a transaction included in a block.
///
/// Returned by `strata_getBlockTransactions`. Carries the computed txid, the
/// target account (if any), constraints, effects, and a tagged
/// [`RpcOLTxKind`] that carries any payload-type-specific fields inline.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLTxDetail {
    /// Computed transaction ID (tree hash of `OLTransactionData`).
    txid: OLTxId,
    /// Target account, or `None` if this transaction type carries no target.
    target: Option<HexBytes32>,
    /// Inclusion constraints.
    constraints: RpcTxConstraints,
    /// Effects produced when this transaction is applied.
    effects: RpcTxEffectsView,
    /// Payload-type discriminator with type-specific fields inline.
    kind: RpcOLTxKind,
}

impl RpcOLTxDetail {
    pub fn txid(&self) -> OLTxId {
        self.txid
    }

    pub fn target(&self) -> Option<&HexBytes32> {
        self.target.as_ref()
    }

    pub fn constraints(&self) -> &RpcTxConstraints {
        &self.constraints
    }

    pub fn effects(&self) -> &RpcTxEffectsView {
        &self.effects
    }

    pub fn kind(&self) -> &RpcOLTxKind {
        &self.kind
    }
}

impl From<&OLTransaction> for RpcOLTxDetail {
    fn from(tx: &OLTransaction) -> Self {
        let txid = tx.compute_txid();
        let data = tx.data();
        let kind = match data.payload() {
            TransactionPayload::GenericAccountMessage(_) => RpcOLTxKind::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate(sau_payload) => {
                RpcOLTxKind::SnarkAccountUpdate(RpcSauTxSummary::from(sau_payload))
            }
        };
        let target = tx.target().map(|a| HexBytes32::from(<[u8; 32]>::from(a)));
        let constraints = RpcTxConstraints::from(data.constraints().clone());
        let effects = RpcTxEffectsView::from(data.effects());
        Self {
            txid,
            target,
            constraints,
            effects,
            kind,
        }
    }
}

/// Payload-type discriminator for [`RpcOLTxDetail`].
///
/// Mirrors the on-chain [`TransactionPayload`] enum and carries any
/// type-specific summary data inline, so the wire format is self-describing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcOLTxKind {
    GenericAccountMessage,
    SnarkAccountUpdate(RpcSauTxSummary),
}

/// Summary of transfers and messages produced by a transaction.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcTxEffectsView {
    transfers: Vec<RpcSentTransfer>,
    messages: Vec<RpcSentMessageEffect>,
}

impl RpcTxEffectsView {
    pub fn transfers(&self) -> &[RpcSentTransfer] {
        &self.transfers
    }

    pub fn messages(&self) -> &[RpcSentMessageEffect] {
        &self.messages
    }
}

impl From<&TxEffects> for RpcTxEffectsView {
    fn from(effects: &TxEffects) -> Self {
        Self {
            transfers: effects
                .transfers_iter()
                .map(RpcSentTransfer::from)
                .collect(),
            messages: effects
                .messages_iter()
                .map(RpcSentMessageEffect::from)
                .collect(),
        }
    }
}

/// Transfer effect: value sent to a destination account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSentTransfer {
    dest: HexBytes32,
    value_sats: u64,
}

impl RpcSentTransfer {
    pub fn dest(&self) -> &HexBytes32 {
        &self.dest
    }

    pub fn value_sats(&self) -> u64 {
        self.value_sats
    }
}

impl From<&SentTransfer> for RpcSentTransfer {
    fn from(xfr: &SentTransfer) -> Self {
        Self {
            dest: HexBytes32::from(<[u8; 32]>::from(xfr.dest())),
            value_sats: xfr.value().to_sat(),
        }
    }
}

/// Message effect: payload sent to a destination account with optional value.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSentMessageEffect {
    dest: HexBytes32,
    value_sats: u64,
    data: HexBytes,
}

impl RpcSentMessageEffect {
    pub fn dest(&self) -> &HexBytes32 {
        &self.dest
    }

    pub fn value_sats(&self) -> u64 {
        self.value_sats
    }

    pub fn data(&self) -> &HexBytes {
        &self.data
    }
}

impl From<&SentMessage> for RpcSentMessageEffect {
    fn from(msg: &SentMessage) -> Self {
        let payload = msg.payload();
        Self {
            dest: HexBytes32::from(<[u8; 32]>::from(msg.dest())),
            value_sats: payload.value().to_sat(),
            data: HexBytes(payload.data().to_vec()),
        }
    }
}

/// SAU-specific summary fields extracted from a snark account update payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSauTxSummary {
    seq_no: u64,
    new_next_msg_idx: u64,
    inner_state_root: HexBytes32,
}

impl RpcSauTxSummary {
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn new_next_msg_idx(&self) -> u64 {
        self.new_next_msg_idx
    }

    pub fn inner_state_root(&self) -> &HexBytes32 {
        &self.inner_state_root
    }
}

impl From<&SauTxPayload> for RpcSauTxSummary {
    fn from(payload: &SauTxPayload) -> Self {
        let update = payload.operation().update();
        let proof_state = update.proof_state();
        Self {
            seq_no: update.seq_no(),
            new_next_msg_idx: proof_state.new_next_msg_idx(),
            inner_state_root: HexBytes32::from(proof_state.inner_state_root().0),
        }
    }
}

impl TryFrom<RpcOLTransaction> for OLTransaction {
    type Error = RpcTxConversionError;

    fn try_from(rpc_tx: RpcOLTransaction) -> Result<Self, Self::Error> {
        let constraints: TxConstraints = rpc_tx.constraints.into();

        match rpc_tx.payload {
            RpcTransactionPayload::GenericAccountMessage(gam) => {
                let target = AccountId::new(gam.target.0);
                let tx_data = OLTransactionData::from_gam_bytes(target, gam.payload.0)?
                    .with_constraints(constraints);
                Ok(OLTransaction::new(tx_data, TxProofs::new_empty()))
            }
            RpcTransactionPayload::SnarkAccountUpdate(sau) => {
                let target = AccountId::new(sau.target().0);

                let operation =
                    UpdateOperationData::from_ssz_bytes(&sau.update_operation_encoded().0)
                        .map_err(|e| RpcTxConversionError::DecodeOperationData(e.to_string()))?;
                let base_update = SnarkAccountUpdate::new(operation, sau.update_proof().0.clone());

                let operation = base_update.operation();
                let proof_state = operation.new_proof_state();
                let sau_proof_state = SauTxProofState::new(
                    proof_state.next_inbox_msg_idx(),
                    proof_state.inner_state(),
                );
                let sau_update_data = SauTxUpdateData::new(
                    operation.seq_no(),
                    sau_proof_state,
                    operation.extra_data().to_vec(),
                );

                let asm_hist_refs = operation.ledger_refs().asm_manifest_refs();
                let sau_ledger_refs = if asm_hist_refs.is_empty() {
                    SauTxLedgerRefs::new_empty()
                } else {
                    let claim_list = ClaimList::new(asm_hist_refs.to_vec())
                        .ok_or(RpcTxConversionError::TooManyAsmHistoryClaims)?;
                    SauTxLedgerRefs::new_with_claims(claim_list)
                };

                let messages = operation.processed_messages().to_vec();
                let sau_operation_data =
                    SauTxOperationData::new(sau_update_data, messages, sau_ledger_refs);
                let payload = TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(
                    target,
                    sau_operation_data,
                ));
                let effects = operation.outputs().to_tx_effects();
                let tx_data =
                    OLTransactionData::new(payload, effects).with_constraints(constraints);
                let tx_proofs = TxProofs::new(
                    ProofSatisfierList::single(base_update.update_proof().to_vec()),
                    None,
                );
                Ok(OLTransaction::new(tx_data, tx_proofs))
            }
        }
    }
}

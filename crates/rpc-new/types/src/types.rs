//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::TransactionAttachment;
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateInputData, UpdateStateData};

/// OL chain status with latest, confirmed, and finalized blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLChainStatus {
    pub latest: OLBlockCommitment,
    pub confirmed: OLBlockCommitment,
    pub finalized: OLBlockCommitment,
}

/// Message payload with bitcoin value and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMsgPayload {
    /// Value in satoshis.
    pub value: u64,
    /// Hex-encoded data.
    #[serde(with = "hex::serde")]
    pub data: Vec<u8>,
}

impl From<MsgPayload> for RpcMsgPayload {
    fn from(payload: MsgPayload) -> Self {
        Self {
            value: payload.value().to_sat(),
            data: payload.data().to_vec(),
        }
    }
}

impl From<RpcMsgPayload> for MsgPayload {
    fn from(rpc: RpcMsgPayload) -> Self {
        MsgPayload::new(BitcoinAmount::from_sat(rpc.value), rpc.data)
    }
}

impl From<&MsgPayload> for RpcMsgPayload {
    fn from(payload: &MsgPayload) -> Self {
        Self {
            value: payload.value().to_sat(),
            data: payload.data().to_vec(),
        }
    }
}

/// Message entry with source account, epoch, and payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMessageEntry {
    #[serde(with = "hex::serde")]
    pub source: [u8; 32],
    pub incl_epoch: u32,
    pub payload: RpcMsgPayload,
}

/// Proof state: inner state commitment and next message index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcProofState {
    #[serde(with = "hex::serde")]
    pub inner_state: [u8; 32],
    pub next_inbox_msg_idx: u64,
}

/// Update state data with proof state and extra data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcUpdateStateData {
    pub proof_state: RpcProofState,
    #[serde(with = "hex::serde")]
    pub extra_data: Vec<u8>,
}

/// Update input data for state transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcUpdateInputData {
    pub seq_no: u64,
    pub messages: Vec<RpcMessageEntry>,
    pub update_state: RpcUpdateStateData,
}

/// Generic account message payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcGenericAccountMessage {
    #[serde(with = "hex::serde")]
    pub target: [u8; 32],
    #[serde(with = "hex::serde")]
    pub payload: Vec<u8>,
}

/// Snark account update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSnarkAccountUpdate {
    #[serde(with = "hex::serde")]
    pub target: [u8; 32],
    pub update: RpcUpdateInputData,
    #[serde(with = "hex::serde")]
    pub update_proof: Vec<u8>,
}

/// Transaction payload: generic message or snark account update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcTransactionPayload {
    GenericAccountMessage(RpcGenericAccountMessage),
    SnarkAccountUpdate(RpcSnarkAccountUpdate),
}

/// Transaction extra: slot constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTransactionAttachment {
    pub min_slot: Option<u64>,
    pub max_slot: Option<u64>,
}

/// OL transaction for submission (excludes accumulator proofs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLTransaction {
    pub payload: RpcTransactionPayload,
    pub attachments: RpcTransactionAttachment,
}

/// Block messages: block ID and its message payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockMessages {
    pub block_id: OLBlockId,
    pub messages: Vec<RpcMsgPayload>,
}

/// Block update inputs: block ID and its update input data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockUpdateInputs {
    pub block_id: OLBlockId,
    pub inputs: Vec<RpcUpdateInputData>,
}

// Type conversions

impl From<MessageEntry> for RpcMessageEntry {
    fn from(entry: MessageEntry) -> Self {
        Self {
            source: *entry.source().inner(),
            incl_epoch: entry.incl_epoch(),
            payload: entry.payload().clone().into(),
        }
    }
}

impl From<RpcMessageEntry> for MessageEntry {
    fn from(rpc: RpcMessageEntry) -> Self {
        MessageEntry::new(
            AccountId::new(rpc.source),
            rpc.incl_epoch,
            rpc.payload.into(),
        )
    }
}

impl From<ProofState> for RpcProofState {
    fn from(state: ProofState) -> Self {
        Self {
            inner_state: state.inner_state(),
            next_inbox_msg_idx: state.next_inbox_msg_idx(),
        }
    }
}

impl From<RpcProofState> for ProofState {
    fn from(rpc: RpcProofState) -> Self {
        ProofState::new(rpc.inner_state, rpc.next_inbox_msg_idx)
    }
}

impl From<RpcUpdateStateData> for UpdateStateData {
    fn from(rpc: RpcUpdateStateData) -> Self {
        UpdateStateData::new(rpc.proof_state.into(), rpc.extra_data)
    }
}

impl From<UpdateInputData> for RpcUpdateInputData {
    fn from(input: UpdateInputData) -> Self {
        Self {
            seq_no: input.seq_no(),
            messages: input
                .processed_messages()
                .iter()
                .map(|m| m.clone().into())
                .collect(),
            update_state: RpcUpdateStateData {
                proof_state: input.new_state().into(),
                extra_data: input.extra_data().to_vec(),
            },
        }
    }
}

impl From<RpcUpdateInputData> for UpdateInputData {
    fn from(rpc: RpcUpdateInputData) -> Self {
        UpdateInputData::new(
            rpc.seq_no,
            rpc.messages.into_iter().map(|m| m.into()).collect(),
            rpc.update_state.into(),
        )
    }
}

impl From<TransactionAttachment> for RpcTransactionAttachment {
    fn from(extra: TransactionAttachment) -> Self {
        Self {
            min_slot: extra.min_slot(),
            max_slot: extra.max_slot(),
        }
    }
}

impl From<RpcTransactionAttachment> for TransactionAttachment {
    fn from(rpc: RpcTransactionAttachment) -> Self {
        TransactionAttachment::new(rpc.min_slot, rpc.max_slot)
    }
}

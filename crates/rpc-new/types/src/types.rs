//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_identifiers::{Buf32, OLBlockCommitment};
use strata_ol_chain_types_new::{Slot, TransactionAttachment};
use strata_primitives::{EpochCommitment, HexBytes, HexBytes32};
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateInputData, UpdateStateData};

/// OL chain status with latest, confirmed, and finalized blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLChainStatus {
    pub latest: OLBlockCommitment,
    pub confirmed: EpochCommitment,
    pub finalized: EpochCommitment,
}

/// Epoch summary for an account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcAccountEpochSummary {
    /// The epoch commitment.
    epoch_commitment: EpochCommitment,
    /// Final balance at the end of the epoch(sats).
    final_balance: u64,
    /// Final sequence number at the end of the epoch.
    final_seq_no: u64,
    /// Final next input idx for the account.
    final_next_input_idx: u64,
    /// All the updates processed in the epoch.
    // NOTE: DA syncing node won't have all the updates for the epoch blocks, it will only have a
    // final update. Maybe it might make sense to have this field as a single RpcUpdateSummary ?
    updates: Vec<RpcUpdateSummary>,
    /// All new messages received in the epoch.
    new_received_messages: Vec<RpcMessageEntry>,
}

/// Describes an update to a snark account. This is derivable from L1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcUpdateSummary {
    /// New state after update.
    new_state: Buf32,
    /// Num messages processed in the update.
    processed_msgs_count: u64,
    /// Next input idx for the account.
    new_next_input_idx: u64,
    /// Any extra data associated with the account.
    extra_data: Vec<u8>,
}

/// Message payload with bitcoin value and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMsgPayload {
    /// Value in satoshis.
    pub value: u64,
    /// Hex-encoded data.
    pub data: HexBytes,
}

impl From<MsgPayload> for RpcMsgPayload {
    fn from(payload: MsgPayload) -> Self {
        let MsgPayload { data, value } = payload;
        let data: Vec<u8> = data.into();
        Self {
            value: value.to_sat(),
            data: data.into(),
        }
    }
}

impl From<RpcMsgPayload> for MsgPayload {
    fn from(rpc: RpcMsgPayload) -> Self {
        MsgPayload::new(BitcoinAmount::from_sat(rpc.value), rpc.data.into())
    }
}

/// Message entry with source account, epoch, and payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMessageEntry {
    /// Sender of the message.
    pub source: HexBytes32,
    /// Epoch that the message was included.
    pub incl_epoch: u32,
    /// Actual message payload.
    pub payload: RpcMsgPayload,
}

/// Proof state: inner state commitment and next message index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcProofState {
    /// The state root.
    pub inner_state: HexBytes32,
    /// Next inbox id to process.
    pub next_inbox_msg_idx: u64,
}

/// Update state data with proof state and extra data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcUpdateStateData {
    pub proof_state: RpcProofState,
    pub extra_data: HexBytes,
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
    pub target: HexBytes32,
    pub payload: HexBytes,
}

/// Snark account update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSnarkAccountUpdate {
    pub target: HexBytes32,
    pub update: RpcUpdateInputData,
    pub update_proof: HexBytes,
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

/// Block data associated with an account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcAccountBlockSummary {
    /// Block commitment.
    block_commitment: OLBlockCommitment,
    /// Block's slot.
    slot: Slot,
    /// Account's balance after the block execution(sats).
    final_balance: u64,
    /// Account's seq no after the block execution.
    final_seq_no: u64,
    /// Account's updates processed in the block.
    updates: Vec<RpcUpdateSummary>,
    /// New messages for the account in the block.
    new_received_messages: Vec<RpcMessageEntry>,
}

// Type conversions

impl From<MessageEntry> for RpcMessageEntry {
    fn from(entry: MessageEntry) -> Self {
        Self {
            source: (*entry.source().inner()).into(),
            incl_epoch: entry.incl_epoch(),
            payload: entry.payload.into(),
        }
    }
}

impl From<RpcMessageEntry> for MessageEntry {
    fn from(rpc: RpcMessageEntry) -> Self {
        MessageEntry::new(
            AccountId::new(rpc.source.0),
            rpc.incl_epoch,
            rpc.payload.into(),
        )
    }
}

impl From<ProofState> for RpcProofState {
    fn from(state: ProofState) -> Self {
        Self {
            inner_state: state.inner_state().into(),
            next_inbox_msg_idx: state.next_inbox_msg_idx(),
        }
    }
}

impl From<RpcProofState> for ProofState {
    fn from(rpc: RpcProofState) -> Self {
        ProofState::new(rpc.inner_state.into(), rpc.next_inbox_msg_idx)
    }
}

impl From<RpcUpdateStateData> for UpdateStateData {
    fn from(rpc: RpcUpdateStateData) -> Self {
        UpdateStateData::new(rpc.proof_state.into(), rpc.extra_data.into())
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
                extra_data: input.extra_data().into(),
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

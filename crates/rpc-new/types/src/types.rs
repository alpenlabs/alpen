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
    /// Latest block commitment.
    latest: OLBlockCommitment,

    /// Confirmed block commitment.
    confirmed: EpochCommitment,

    /// Finalized block commitment.
    finalized: EpochCommitment,
}

impl RpcOLChainStatus {
    /// Creates a new [`RpcOLChainStatus`].
    pub fn new(
        latest: OLBlockCommitment,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            latest,
            confirmed,
            finalized,
        }
    }

    /// Returns the latest block commitment.
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }

    /// Returns the confirmed block commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized block commitment.
    pub fn finalized(&self) -> &EpochCommitment {
        &self.finalized
    }
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

impl RpcAccountEpochSummary {
    /// Creates a new [`RpcAccountEpochSummary`].
    pub fn new(
        epoch_commitment: EpochCommitment,
        final_balance: u64,
        final_seq_no: u64,
        final_next_input_idx: u64,
        updates: Vec<RpcUpdateSummary>,
        new_received_messages: Vec<RpcMessageEntry>,
    ) -> Self {
        Self {
            epoch_commitment,
            final_balance,
            final_seq_no,
            final_next_input_idx,
            updates,
            new_received_messages,
        }
    }

    /// Returns the epoch commitment.
    pub fn epoch_commitment(&self) -> &EpochCommitment {
        &self.epoch_commitment
    }

    /// Returns the final balance.
    pub fn final_balance(&self) -> u64 {
        self.final_balance
    }

    /// Returns the final sequence number.
    pub fn final_seq_no(&self) -> u64 {
        self.final_seq_no
    }

    /// Returns the final next input index.
    pub fn final_next_input_idx(&self) -> u64 {
        self.final_next_input_idx
    }

    /// Returns the updates.
    pub fn updates(&self) -> &[RpcUpdateSummary] {
        &self.updates
    }

    /// Returns the new received messages.
    pub fn new_received_messages(&self) -> &[RpcMessageEntry] {
        &self.new_received_messages
    }
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

impl RpcUpdateSummary {
    /// Creates a new [`RpcUpdateSummary`].
    pub fn new(
        new_state: Buf32,
        processed_msgs_count: u64,
        new_next_input_idx: u64,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            new_state,
            processed_msgs_count,
            new_next_input_idx,
            extra_data,
        }
    }

    /// Returns the new state.
    pub fn new_state(&self) -> &Buf32 {
        &self.new_state
    }

    /// Returns the number of processed messages.
    pub fn processed_msgs_count(&self) -> u64 {
        self.processed_msgs_count
    }

    /// Returns the new next input index.
    pub fn new_next_input_idx(&self) -> u64 {
        self.new_next_input_idx
    }

    /// Returns the extra data.
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

/// Message payload with bitcoin value and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMsgPayload {
    /// Value in satoshis.
    value: u64,

    /// Hex-encoded data.
    data: HexBytes,
}

impl RpcMsgPayload {
    /// Creates a new [`RpcMsgPayload`].
    pub fn new(value: u64, data: HexBytes) -> Self {
        Self { value, data }
    }

    /// Returns the value.
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Returns the data.
    pub fn data(&self) -> &HexBytes {
        &self.data
    }
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
    source: HexBytes32,

    /// Epoch that the message was included.
    incl_epoch: u32,

    /// Actual message payload.
    payload: RpcMsgPayload,
}

impl RpcMessageEntry {
    /// Creates a new [`RpcMessageEntry`].
    pub fn new(source: HexBytes32, incl_epoch: u32, payload: RpcMsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    /// Returns the source.
    pub fn source(&self) -> &HexBytes32 {
        &self.source
    }

    /// Returns the inclusion epoch.
    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    /// Returns the payload.
    pub fn payload(&self) -> &RpcMsgPayload {
        &self.payload
    }
}

/// Proof state: inner state commitment and next message index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcProofState {
    /// The state root.
    inner_state: HexBytes32,

    /// Next inbox id to process.
    next_inbox_msg_idx: u64,
}

impl RpcProofState {
    /// Creates a new [`RpcProofState`].
    pub fn new(inner_state: HexBytes32, next_inbox_msg_idx: u64) -> Self {
        Self {
            inner_state,
            next_inbox_msg_idx,
        }
    }

    /// Returns the inner state.
    pub fn inner_state(&self) -> &HexBytes32 {
        &self.inner_state
    }

    /// Returns the next inbox message index.
    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}

/// Update state data with proof state and extra data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcUpdateStateData {
    /// The proof state.
    proof_state: RpcProofState,

    /// The extra data.
    extra_data: HexBytes,
}

impl RpcUpdateStateData {
    /// Creates a new [`RpcUpdateStateData`].
    pub fn new(proof_state: RpcProofState, extra_data: HexBytes) -> Self {
        Self {
            proof_state,
            extra_data,
        }
    }

    /// Returns the proof state.
    pub fn proof_state(&self) -> &RpcProofState {
        &self.proof_state
    }

    /// Returns the extra data.
    pub fn extra_data(&self) -> &HexBytes {
        &self.extra_data
    }
}

/// Update input data for state transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcUpdateInputData {
    /// The sequence number.
    seq_no: u64,

    /// The messages.
    messages: Vec<RpcMessageEntry>,

    /// The update state.
    update_state: RpcUpdateStateData,
}

impl RpcUpdateInputData {
    /// Creates a new [`RpcUpdateInputData`].
    pub fn new(
        seq_no: u64,
        messages: Vec<RpcMessageEntry>,
        update_state: RpcUpdateStateData,
    ) -> Self {
        Self {
            seq_no,
            messages,
            update_state,
        }
    }

    /// Returns the sequence number.
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Returns the messages.
    pub fn messages(&self) -> &[RpcMessageEntry] {
        &self.messages
    }

    /// Returns the update state.
    pub fn update_state(&self) -> &RpcUpdateStateData {
        &self.update_state
    }
}

/// Generic account message payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Snark account update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSnarkAccountUpdate {
    /// The target account.
    target: HexBytes32,

    /// The update input data.
    update: RpcUpdateInputData,

    /// The update proof.
    update_proof: HexBytes,
}

impl RpcSnarkAccountUpdate {
    /// Creates a new [`RpcSnarkAccountUpdate`].
    pub fn new(target: HexBytes32, update: RpcUpdateInputData, update_proof: HexBytes) -> Self {
        Self {
            target,
            update,
            update_proof,
        }
    }

    /// Returns the target account.
    pub fn target(&self) -> &HexBytes32 {
        &self.target
    }

    /// Returns the update input data.
    pub fn update(&self) -> &RpcUpdateInputData {
        &self.update
    }

    /// Returns the update proof.
    pub fn update_proof(&self) -> &HexBytes {
        &self.update_proof
    }
}

/// Transaction payload: generic message or snark account update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcTransactionPayload {
    /// Generic account message.
    GenericAccountMessage(RpcGenericAccountMessage),

    /// Snark account update.
    SnarkAccountUpdate(RpcSnarkAccountUpdate),
}

/// Transaction extra: slot constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTransactionAttachment {
    /// Minimum slot.
    min_slot: Option<u64>,

    /// Maximum slot.
    max_slot: Option<u64>,
}

impl RpcTransactionAttachment {
    /// Creates a new [`RpcTransactionAttachment`].
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

/// OL transaction for submission (excludes accumulator proofs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLTransaction {
    /// The payload.
    payload: RpcTransactionPayload,

    /// The attachments.
    attachments: RpcTransactionAttachment,
}

impl RpcOLTransaction {
    /// Creates a new [`RpcOLTransaction`].
    pub fn new(payload: RpcTransactionPayload, attachments: RpcTransactionAttachment) -> Self {
        Self {
            payload,
            attachments,
        }
    }

    /// Returns the payload.
    pub fn payload(&self) -> &RpcTransactionPayload {
        &self.payload
    }

    /// Returns the attachments.
    pub fn attachments(&self) -> &RpcTransactionAttachment {
        &self.attachments
    }
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

impl RpcAccountBlockSummary {
    /// Creates a new [`RpcAccountBlockSummary`].
    pub fn new(
        block_commitment: OLBlockCommitment,
        slot: Slot,
        final_balance: u64,
        final_seq_no: u64,
        updates: Vec<RpcUpdateSummary>,
        new_received_messages: Vec<RpcMessageEntry>,
    ) -> Self {
        Self {
            block_commitment,
            slot,
            final_balance,
            final_seq_no,
            updates,
            new_received_messages,
        }
    }

    /// Returns the block commitment.
    pub fn block_commitment(&self) -> &OLBlockCommitment {
        &self.block_commitment
    }

    /// Returns the slot.
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    /// Returns the final balance.
    pub fn final_balance(&self) -> u64 {
        self.final_balance
    }

    /// Returns the final sequence number.
    pub fn final_seq_no(&self) -> u64 {
        self.final_seq_no
    }

    /// Returns the updates.
    pub fn updates(&self) -> &[RpcUpdateSummary] {
        &self.updates
    }

    /// Returns the new received messages.
    pub fn new_received_messages(&self) -> &[RpcMessageEntry] {
        &self.new_received_messages
    }
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

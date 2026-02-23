//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_identifiers::OLBlockCommitment;
use strata_primitives::{EpochCommitment, HexBytes, HexBytes32};
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateInputData, UpdateStateData};

/// Summary for an account's data for an epoch.
/// This information can be reconstructed fully from data in DA.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountEpochSummary {
    /// The epoch commitment.
    pub epoch_commitment: EpochCommitment,
    /// Previous epoch commitment.
    pub prev_epoch_commitment: EpochCommitment,
    /// Balance of account at the end of this epoch in sats.
    pub balance: u64,
    /// Next expected sequence number for account at the end of this epoch.
    pub next_seq_no: u64,
    /// Account state at end of this epoch.
    pub proof_state: RpcProofState,
    /// Final extra data sent in updates to account during this epoch.
    pub extra_data: HexBytes,
    /// Messages processed by account in this epoch.
    pub processed_msgs: Vec<RpcMessageEntry>,
}

impl RpcAccountEpochSummary {
    /// Creates a new [`RpcAccountEpochSummary`].
    pub fn new(
        epoch_commitment: EpochCommitment,
        prev_epoch_commitment: EpochCommitment,
        balance: u64,
        next_seq_no: u64,
        proof_state: ProofState,
        extra_data: Vec<u8>,
        processed_msgs: Vec<MessageEntry>,
    ) -> Self {
        Self {
            epoch_commitment,
            prev_epoch_commitment,
            balance,
            next_seq_no,
            proof_state: proof_state.into(),
            extra_data: extra_data.into(),
            processed_msgs: processed_msgs.into_iter().map(Into::into).collect(),
        }
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    pub fn prev_epoch(&self) -> EpochCommitment {
        self.prev_epoch_commitment
    }

    pub fn balance(&self) -> u64 {
        self.balance
    }

    pub fn next_seq_no(&self) -> u64 {
        self.next_seq_no
    }

    /// Returns the new state.
    pub fn proof_state(&self) -> &RpcProofState {
        &self.proof_state
    }

    /// Returns the extra data.
    pub fn extra_data(&self) -> &HexBytes {
        &self.extra_data
    }

    /// Returns the processed messages in this epoch.
    pub fn processed_msgs(&self) -> &[RpcMessageEntry] {
        &self.processed_msgs
    }
}

/// RPC serializable account data at given ol block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountBlockSummary {
    /// Account Id
    pub account: HexBytes32,
    /// Block commitment.
    pub block_commitment: OLBlockCommitment,
    /// Balance of account after block execution in sats.
    pub balance: u64,
    /// Next expected sequence number for account after block execution.
    pub next_seq_no: u64,
    /// Account's updates processed in the block.
    pub updates: Vec<RpcUpdateInputData>,
    /// New messages added to account's inbox in this block.
    pub new_inbox_messages: Vec<RpcMessageEntry>,
    /// Next expected message inbox accumulator index after block execution.
    pub next_inbox_msg_idx: u64,
}

impl RpcAccountBlockSummary {
    /// Creates a new [`RpcAccountBlockSummary`].
    pub fn new(
        account: AccountId,
        block_commitment: OLBlockCommitment,
        balance: BitcoinAmount,
        next_seq_no: u64,
        updates: Vec<UpdateInputData>,
        new_inbox_messages: Vec<MessageEntry>,
        next_inbox_msg_idx: u64,
    ) -> Self {
        Self {
            account: account.into_inner().into(),
            block_commitment,
            balance: balance.to_sat(),
            next_seq_no,
            updates: updates.into_iter().map(Into::into).collect(),
            new_inbox_messages: new_inbox_messages.into_iter().map(Into::into).collect(),
            next_inbox_msg_idx,
        }
    }

    /// Returns the account id
    pub fn account(&self) -> &HexBytes32 {
        &self.account
    }

    /// Returns the commitment to this block.
    pub fn block_commitment(&self) -> &OLBlockCommitment {
        &self.block_commitment
    }

    /// Returns the balance of account after block execution in sats.
    pub fn balance(&self) -> u64 {
        self.balance
    }

    /// Returns the next expected sequence number for account after block execution.
    pub fn next_seq_no(&self) -> u64 {
        self.next_seq_no
    }

    /// Returns the updates for account processed in this block.
    pub fn updates(&self) -> &[RpcUpdateInputData] {
        &self.updates
    }

    /// Returns the new messages added to account's inbox in this block.
    pub fn new_inbox_messages(&self) -> &[RpcMessageEntry] {
        &self.new_inbox_messages
    }

    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}

/// RPC serializable version of [`UpdateInputData`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcUpdateInputData {
    /// Sequence number of the update.
    seq_no: u64,
    /// Expected final state after update.
    proof_state: RpcProofState,
    /// Extra data posted with this update.
    extra_data: HexBytes,
    /// Account inbox messages processed in this update.
    messages: Vec<RpcMessageEntry>,
}

impl From<UpdateInputData> for RpcUpdateInputData {
    fn from(value: UpdateInputData) -> Self {
        Self {
            seq_no: value.seq_no,
            proof_state: value.update_state.proof_state.into(),
            extra_data: value.update_state.extra_data.to_vec().into(),
            messages: value.messages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RpcUpdateInputData> for UpdateInputData {
    fn from(rpc: RpcUpdateInputData) -> Self {
        UpdateInputData::new(
            rpc.seq_no,
            rpc.messages.into_iter().map(Into::into).collect(),
            UpdateStateData::new(rpc.proof_state.into(), rpc.extra_data.0),
        )
    }
}

/// RPC serializable version of [`MessageEntry`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcMessageEntry {
    /// Sender of the message.
    source: HexBytes32,
    /// Epoch that the message was included.
    incl_epoch: u32,
    /// Actual message payload.
    payload: RpcMsgPayload,
}

impl From<MessageEntry> for RpcMessageEntry {
    fn from(entry: MessageEntry) -> Self {
        Self {
            source: <[u8; 32]>::from(entry.source).into(),
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

/// RPC serializable version of [`MsgPayload`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcMsgPayload {
    /// Value in sats.
    value: u64,
    /// Hex-encoded data.
    data: HexBytes,
}

impl From<MsgPayload> for RpcMsgPayload {
    fn from(payload: MsgPayload) -> Self {
        Self {
            value: payload.value.to_sat(),
            data: payload.data.to_vec().into(),
        }
    }
}

impl From<RpcMsgPayload> for MsgPayload {
    fn from(rpc: RpcMsgPayload) -> Self {
        MsgPayload::new(BitcoinAmount::from_sat(rpc.value), rpc.data.into())
    }
}

/// RPC serializable version of [`ProofState`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcProofState {
    /// The state root.
    inner_state: HexBytes32,
    /// Next inbox id to process.
    next_inbox_msg_idx: u64,
}

impl From<ProofState> for RpcProofState {
    fn from(state: ProofState) -> Self {
        Self {
            inner_state: state.inner_state().0.into(),
            next_inbox_msg_idx: state.next_inbox_msg_idx(),
        }
    }
}

impl From<RpcProofState> for ProofState {
    fn from(rpc: RpcProofState) -> Self {
        ProofState::new(rpc.inner_state.0.into(), rpc.next_inbox_msg_idx)
    }
}

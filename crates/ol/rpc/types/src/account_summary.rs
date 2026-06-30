//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use strata_acct_types::{AccountId, BitcoinAmount, MessageEntry, MsgPayload, MsgPayloadError};
use strata_db_types::ol_state_index::AccountUpdateRecord;
use strata_identifiers::OLBlockCommitment;
use strata_primitives::{EpochCommitment, HexBytes, HexBytes32};
use strata_snark_acct_types::{ProofState, UpdateInputData};

/// Summary for an account's data for an epoch.
/// This information can be reconstructed fully from data in DA.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountEpochSummary {
    /// The epoch commitment.
    epoch_commitment: EpochCommitment,
    /// Previous epoch commitment.
    prev_epoch_commitment: EpochCommitment,
    /// Balance of account at the end of this epoch in sats.
    final_balance: u64,
    /// Account's state root at the end of this epoch.
    final_state_root: HexBytes32,
    /// Update inputs for this epoch if present
    update_inputs: Vec<RpcUpdateInputData>,
}

impl RpcAccountEpochSummary {
    /// Creates a new [`RpcAccountEpochSummary`].
    pub fn new(
        epoch_commitment: EpochCommitment,
        prev_epoch_commitment: EpochCommitment,
        final_balance: u64,
        final_state_root: HexBytes32,
        update_inputs: Vec<RpcUpdateInputData>,
    ) -> Self {
        Self {
            epoch_commitment,
            prev_epoch_commitment,
            final_balance,
            final_state_root,
            update_inputs,
        }
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    pub fn prev_epoch(&self) -> EpochCommitment {
        self.prev_epoch_commitment
    }

    pub fn final_balance(&self) -> u64 {
        self.final_balance
    }

    pub fn final_state_root(&self) -> &HexBytes32 {
        &self.final_state_root
    }

    pub fn update_inputs(&self) -> &[RpcUpdateInputData] {
        &self.update_inputs
    }

    pub fn epoch_commitment(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    pub fn prev_epoch_commitment(&self) -> EpochCommitment {
        self.prev_epoch_commitment
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
    pub seq_no: u64,
    /// Inbox cursor after this update.
    pub next_inbox_msg_idx: u64,
    /// Inner state root after this update. `None` on checkpoint-sync nodes,
    /// which do not store per-update roots (only the post-epoch root on the
    /// epoch summary). The epoch's `final_state_root` is still populated.
    pub new_state_root: Option<HexBytes32>,
    /// Extra data posted with this update.
    pub extra_data: HexBytes,
    /// Account inbox messages processed in this update.
    pub messages: Vec<RpcMessageEntry>,
}

/// Published manifest metadata for a Snark account update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSnarkAcctUpdateManifest {
    /// Sequence number of the update.
    seq_no: u64,
    /// Inner state root after this update, if stored by the serving node.
    new_inner_state_root: Option<HexBytes32>,
    /// Inbox cursor before this update.
    prev_next_msg_idx: u64,
    /// Inbox cursor after this update.
    new_next_msg_idx: u64,
    /// Extra data posted with this update, if stored by the serving node.
    extra_data: Option<HexBytes>,
}

impl RpcSnarkAcctUpdateManifest {
    /// Creates a new [`RpcSnarkAcctUpdateManifest`].
    pub fn new(
        seq_no: u64,
        new_inner_state_root: Option<HexBytes32>,
        prev_next_msg_idx: u64,
        new_next_msg_idx: u64,
        extra_data: Option<HexBytes>,
    ) -> Self {
        Self {
            seq_no,
            new_inner_state_root,
            prev_next_msg_idx,
            new_next_msg_idx,
            extra_data,
        }
    }

    /// Creates a manifest response from an account update record.
    pub fn from_account_update_record(record: &AccountUpdateRecord, operation_seq_no: u64) -> Self {
        Self {
            seq_no: operation_seq_no,
            new_inner_state_root: record
                .update_meta()
                .map(|meta| HexBytes32::from(*meta.new_state_root().as_ref())),
            prev_next_msg_idx: record.prev_next_inbox_idx(),
            new_next_msg_idx: record.next_inbox_idx(),
            extra_data: record
                .extra_data()
                .map(|data| HexBytes::from(data.to_vec())),
        }
    }

    /// Returns the update sequence number.
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Returns the inner state root after this update, if available.
    pub fn new_inner_state_root(&self) -> Option<&HexBytes32> {
        self.new_inner_state_root.as_ref()
    }

    /// Returns the inbox cursor before this update.
    pub fn prev_next_msg_idx(&self) -> u64 {
        self.prev_next_msg_idx
    }

    /// Returns the inbox cursor after this update.
    pub fn new_next_msg_idx(&self) -> u64 {
        self.new_next_msg_idx
    }

    /// Returns the update extra data, if available.
    pub fn extra_data(&self) -> Option<&HexBytes> {
        self.extra_data.as_ref()
    }
}

impl From<UpdateInputData> for RpcUpdateInputData {
    fn from(value: UpdateInputData) -> Self {
        let proof_state = value.update_state.proof_state;
        Self {
            seq_no: value.seq_no,
            next_inbox_msg_idx: proof_state.next_inbox_msg_idx(),
            new_state_root: Some(proof_state.inner_state().0.into()),
            extra_data: value.update_state.extra_data.to_vec().into(),
            messages: value.messages.into_iter().map(Into::into).collect(),
        }
    }
}

/// RPC serializable value with its absolute index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcIndexedEntry<T> {
    /// Absolute index of the value.
    index: u64,
    /// Value at `index`.
    value: T,
}

impl<T> RpcIndexedEntry<T> {
    /// Creates a new [`RpcIndexedEntry`].
    pub fn new(index: u64, value: T) -> Self {
        Self { index, value }
    }

    /// Returns the absolute index.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Returns the indexed value.
    pub fn value(&self) -> &T {
        &self.value
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

impl TryFrom<RpcMessageEntry> for MessageEntry {
    type Error = MsgPayloadError;

    fn try_from(rpc: RpcMessageEntry) -> Result<Self, Self::Error> {
        Ok(MessageEntry::new(
            AccountId::new(rpc.source.0),
            rpc.incl_epoch,
            rpc.payload.try_into()?,
        ))
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

impl TryFrom<RpcMsgPayload> for MsgPayload {
    type Error = MsgPayloadError;

    fn try_from(rpc: RpcMsgPayload) -> Result<Self, Self::Error> {
        MsgPayload::from_bytes(BitcoinAmount::from_sat(rpc.value), rpc.data.into())
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

impl RpcProofState {
    /// Returns the state root.
    pub fn inner_state(&self) -> &HexBytes32 {
        &self.inner_state
    }

    /// Returns the next inbox message index.
    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}

impl From<RpcProofState> for ProofState {
    fn from(rpc: RpcProofState) -> Self {
        ProofState::new(rpc.inner_state.0.into(), rpc.next_inbox_msg_idx)
    }
}

//! Data structures for that represents the JSON responses. `rpc` crate should depend on this.
//!
//!  Following the <https://github.com/rust-bitcoin/rust-bitcoincore-rpc> where there are separate crates for
//!  - implementation of RPC client
//!  - crate for just data structures that represents the JSON responses from Bitcoin core RPC

use bitcoin::{BlockHash, Network, Txid, Wtxid};
use serde::{Deserialize, Serialize};
use strata_bridge_types::{DepositEntry, DepositState, OperatorIdx, WithdrawalIntent};
use strata_checkpoint_types::BatchInfo;
use strata_csm_types::{CheckpointL1Ref, L1Status};
use strata_db::types::{CheckpointConfStatus, CheckpointEntry};
use strata_ol_chain_types::L2BlockId;
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    buf::Buf32,
    epoch::EpochCommitment,
    l1::{BitcoinAmount, L1BlockCommitment, OutputRef},
    l2::L2BlockCommitment,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexBytes(#[serde(with = "hex::serde")] pub Vec<u8>);

impl HexBytes {
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for HexBytes {
    fn from(value: Vec<u8>) -> Self {
        HexBytes(value)
    }
}

impl From<&[u8]> for HexBytes {
    fn from(value: &[u8]) -> Self {
        HexBytes(value.to_vec())
    }
}

impl From<Box<[u8]>> for HexBytes {
    fn from(value: Box<[u8]>) -> Self {
        HexBytes(value.into_vec())
    }
}

impl From<HexBytes> for Vec<u8> {
    fn from(value: HexBytes) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexBytes32(#[serde(with = "hex::serde")] pub [u8; 32]);

impl From<&L2BlockId> for HexBytes32 {
    fn from(value: &L2BlockId) -> Self {
        Self(*value.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexBytes64(#[serde(with = "hex::serde")] pub [u8; 64]);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcL1Status {
    /// If the last time we tried to poll the client (as of `last_update`)
    /// we were successful.
    pub bitcoin_rpc_connected: bool,

    /// The last error message we received when trying to poll the client, if
    /// there was one.
    pub last_rpc_error: Option<String>,

    /// Current block height.
    pub cur_height: u64,

    /// Current tip block ID as string.
    pub cur_tip_blkid: String,

    /// Last published txid where L2 blob was present
    pub last_published_txid: Option<Txid>,

    /// number of published transactions in current run (commit + reveal pair count as 1)
    pub published_envelope_count: u64,

    /// UNIX millis time of the last time we got a new update from the L1 connector.
    pub last_update: u64,

    /// Underlying network.
    pub network: Network,
}

impl RpcL1Status {
    pub fn from_l1_status(l1s: L1Status, network: Network) -> Self {
        Self {
            bitcoin_rpc_connected: l1s.bitcoin_rpc_connected,
            last_rpc_error: l1s.last_rpc_error,
            cur_height: l1s.cur_height,
            cur_tip_blkid: l1s.cur_tip_blkid,
            last_published_txid: l1s.last_published_txid.map(Into::into),
            published_envelope_count: l1s.published_reveal_txs_count,
            last_update: l1s.last_update,
            network,
        }
    }
}

impl Default for RpcL1Status {
    fn default() -> Self {
        Self {
            bitcoin_rpc_connected: Default::default(),
            last_rpc_error: Default::default(),
            cur_height: Default::default(),
            cur_tip_blkid: Default::default(),
            last_published_txid: Default::default(),
            published_envelope_count: Default::default(),
            last_update: Default::default(),
            network: Network::Regtest,
        }
    }
}

/// In reference to checkpointed client state tracked by the CSM.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcClientStatus {
    /// Epoch that's been confirmed and buried on L1 and we can assume won't
    /// roll back.
    pub finalized_epoch: Option<EpochCommitment>,

    /// Epoch that's been confirmed on L1 but might still roll back.
    pub confirmed_epoch: Option<EpochCommitment>,

    /// Tip L1 block that we're following.
    pub tip_l1_block: Option<L1BlockCommitment>,

    /// Buried L1 block that we use to determine the finalized epoch.
    pub buried_l1_block: Option<L1BlockCommitment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcBlockHeader {
    /// The index of the block representing height.
    pub block_idx: u64,

    /// Epoch a block belongs to.
    pub epoch: u64,

    /// The timestamp of when the block was created in UNIX epoch format.
    pub timestamp: u64,

    /// hash of the block's contents.
    #[serde(with = "hex::serde")]
    pub block_id: [u8; 32],

    /// previous block
    #[serde(with = "hex::serde")]
    pub prev_block: [u8; 32],

    // L1 segment hash
    #[serde(with = "hex::serde")]
    pub l1_segment_hash: [u8; 32],

    /// Hash of the execution segment
    #[serde(with = "hex::serde")]
    pub exec_segment_hash: [u8; 32],

    /// The root hash of the state tree
    #[serde(with = "hex::serde")]
    pub state_root: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaBlob {
    /// The destination or identifier for the blob.
    pub dest: u8,

    ///  The commitment hash for blob
    pub blob_commitment: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcExecUpdate {
    /// The index of the update, used to track or sequence updates.
    pub update_idx: u64,

    /// Merkle tree root of the contents of the EL payload, in the order it was
    /// strataed in the block.
    #[serde(with = "hex::serde")]
    pub entries_root: [u8; 32],

    /// Buffer of any other payload data.  This is used with the other fields
    /// here to construct the full EVM header payload.
    #[serde(with = "hex::serde")]
    pub extra_payload: Vec<u8>,

    /// New state root for the update.  This is not just the inner EL payload,
    /// but also any extra bookkeeping we need across multiple.
    #[serde(with = "hex::serde")]
    pub new_state: [u8; 32],

    /// Bridge withdrawal intents.
    pub withdrawals: Vec<WithdrawalIntent>,

    /// DA blobs that we expect to see on L1.  This may be empty, probably is
    /// only set near the end of the range of blocks in a batch since we only
    /// assert these in a per-batch frequency.
    pub da_blobs: Vec<DaBlob>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcSyncStatus {
    /// Current head L2 slot known to this node
    // TODO consolidate into using L2BlockCommitment
    pub tip_height: u64,

    /// Last L2 block we've chosen as the current tip.
    // TODO consolidate into using L2BlockCommitment
    pub tip_block_id: L2BlockId,

    /// Current epoch from chainstate.
    pub cur_epoch: u64,

    /// Previous epoch from chainstate.
    pub prev_epoch: EpochCommitment,

    /// Observed finalized epoch from chainstate.
    pub observed_finalized_epoch: EpochCommitment,

    /// Most recent L1 block we've acted on on-chain.
    pub safe_l1_block: L1BlockCommitment,

    /// Terminal blkid of observed finalized epoch from chainstate.
    ///
    /// Note that this is not necessarily the most recently finalized epoch,
    /// it's the one we've also observed, so it's behind by >~1.
    ///
    /// If you want the real one from L1, use another method.
    // TODO which other method?
    #[deprecated]
    pub finalized_block_id: L2BlockId,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawBlockWitness {
    pub raw_l2_block: Vec<u8>,
    pub raw_chain_state: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpcCheckpointConfStatus {
    /// Pending to be posted on L1
    Pending,
    /// Confirmed on L1
    Confirmed,
    /// Finalized on L1
    Finalized,
}

impl From<CheckpointConfStatus> for RpcCheckpointConfStatus {
    fn from(value: CheckpointConfStatus) -> Self {
        match value {
            CheckpointConfStatus::Pending => Self::Pending,
            CheckpointConfStatus::Confirmed(_) => Self::Confirmed,
            CheckpointConfStatus::Finalized(_) => Self::Finalized,
        }
    }
}

impl From<CheckpointEntry> for RpcCheckpointConfStatus {
    fn from(value: CheckpointEntry) -> Self {
        value.confirmation_status.into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcCheckpointInfo {
    /// The index of the checkpoint
    pub idx: u64,
    /// L1 range  the checkpoint covers
    pub l1_range: (L1BlockCommitment, L1BlockCommitment),
    /// L2 range the checkpoint covers
    pub l2_range: (L2BlockCommitment, L2BlockCommitment),
    /// Info on txn where checkpoint is committed on chain
    pub l1_reference: Option<RpcCheckpointL1Ref>,
    /// Confirmation status of checkpoint
    pub confirmation_status: RpcCheckpointConfStatus,
}

impl From<BatchInfo> for RpcCheckpointInfo {
    fn from(value: BatchInfo) -> Self {
        Self {
            idx: value.epoch,
            l1_range: value.l1_range,
            l2_range: value.l2_range,
            l1_reference: None,
            confirmation_status: RpcCheckpointConfStatus::Pending,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcCheckpointL1Ref {
    pub block_height: u64,
    pub block_id: BlockHash,
    pub txid: Txid,
    pub wtxid: Wtxid,
}

impl From<CheckpointL1Ref> for RpcCheckpointL1Ref {
    fn from(l1ref: CheckpointL1Ref) -> Self {
        Self {
            block_height: l1ref.l1_commitment.height_u64(),
            block_id: (*l1ref.l1_commitment.blkid()).into(),
            txid: l1ref.txid.into(),
            wtxid: l1ref.wtxid.into(),
        }
    }
}

impl From<CheckpointEntry> for RpcCheckpointInfo {
    fn from(value: CheckpointEntry) -> Self {
        let mut item: Self = value.checkpoint.batch_info().clone().into();
        item.l1_reference = match value.confirmation_status.clone() {
            CheckpointConfStatus::Pending => None,
            CheckpointConfStatus::Confirmed(lref) => Some(lref.into()),
            CheckpointConfStatus::Finalized(lref) => Some(lref.into()),
        };
        item.confirmation_status = value.confirmation_status.into();
        item
    }
}

/// Withdrawal assignment entry for RPC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcWithdrawalAssignment {
    /// Corresponding deposit id
    pub deposit_idx: u32,
    /// Corresponding deposit txid
    pub deposit_txid: Txid,
    /// Quantity of L1 asset, for Bitcoin this is sats
    pub amt: BitcoinAmount,
    /// Destination [`Descriptor`] for the withdrawal
    pub destination: Descriptor,
    /// operator index
    pub operator_idx: OperatorIdx,
}

/// Deposit entry for RPC corresponding to [`DepositEntry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcDepositEntry {
    deposit_idx: u32,

    /// The outpoint that this deposit entry references.
    output: OutputRef,

    /// List of notary operators, by their indexes.
    // TODO convert this to a windowed bitmap or something
    notary_operators: Vec<OperatorIdx>,

    /// Deposit amount, in the native asset.
    amt: BitcoinAmount,

    /// Deposit state.
    state: DepositState,

    /// Withdrawal request transaction id
    withdrawal_request_txid: Option<Buf32>,
}

impl RpcDepositEntry {
    pub fn from_deposit_entry(ent: &DepositEntry) -> Self {
        Self {
            deposit_idx: ent.idx(),
            output: *ent.output(),
            notary_operators: ent.notary_operators().to_vec(),
            amt: ent.amt(),
            state: ent.deposit_state().clone(),
            withdrawal_request_txid: ent.withdrawal_request_txid(),
        }
    }
}

/// status of L2 Block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum L2BlockStatus {
    /// Unknown block height
    Unknown,
    /// Block is received and present in the longest chain
    Confirmed,
    /// Block is now conformed on L1, and present at certain L1 height
    Verified(u64),
    /// Block is now finalized, certain depth has been reached in L1
    Finalized(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcChainState {
    /// Most recent seen block.
    pub tip_blkid: L2BlockId,

    /// The slot of the last produced block.
    pub tip_slot: u64,

    pub cur_epoch: u64,
}

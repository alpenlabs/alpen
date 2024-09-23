use alpen_express_primitives::prelude::*;
use alpen_express_state::{block::L2BlockBundle, exec_update::ExecUpdate, id::L2BlockId};

/// Succinct commitment to relevant EL block data.
// This ended up being the same as the EL payload types in the state crate,
// should we consolidate?
#[derive(Clone, Debug)]
pub struct ExecPayloadData {
    /// Encoded EL payload, minus any operations we push to it.
    ///
    /// This is the "explicit" input from the CL block.
    exec_update: ExecUpdate,

    accessory_data: Vec<u8>,

    /// CL operations pushed into the EL, such as deposits from L1.  This
    /// corresponds to the "withdrawals" field in the `ExecutionPayloadVX`
    /// type(s), but is separated here because we control it ourselves.
    ///
    /// This is an "implicit" input from elsewhere in the CL STF.
    ops: Vec<Op>,
}

impl ExecPayloadData {
    pub fn new(exec_update: ExecUpdate, accessory_data: Vec<u8>, ops: Vec<Op>) -> Self {
        Self {
            exec_update,
            accessory_data,
            ops,
        }
    }

    pub fn from_l2_block_bundle(l2block: &L2BlockBundle) -> Self {
        Self {
            exec_update: l2block.block().exec_segment().update().clone(),
            accessory_data: l2block.accessory().exec_payload().to_vec(),
            // TODO: extract ops from block
            ops: vec![],
        }
    }

    pub fn exec_update(&self) -> &ExecUpdate {
        &self.exec_update
    }

    pub fn accessory_data(&self) -> &[u8] {
        &self.accessory_data
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
}

/// L1 withdrawal data.
#[derive(Clone, Debug)]
pub struct WithdrawData {
    /// Amount in L1 native asset.  For Bitcoin this is sats.
    _amt: u64,

    /// Schnorr pubkey for the taproot output we're going to generate.
    _dest_addr: Buf64,
}

/// Environment state from the CL that we pass into the EL for the payload we're
/// producing.  Maybe this should also have L1 headers or something?
#[derive(Clone, Debug)]
pub struct PayloadEnv {
    /// Timestamp we're attesting this block was created on.
    timestamp: u64,

    /// BlockId of previous CL block
    prev_l2_block_id: L2BlockId,

    /// Safe L1 block we're exposing into the EL that's not likely to reorg.
    _safe_l1_block: Buf32,

    /// Operations we're pushing into the EL for processing.
    el_ops: Vec<Op>,
}

impl PayloadEnv {
    pub fn new(
        timestamp: u64,
        prev_l2_block_id: L2BlockId,
        _safe_l1_block: Buf32,
        el_ops: Vec<Op>,
    ) -> Self {
        Self {
            timestamp,
            prev_l2_block_id,
            _safe_l1_block,
            el_ops,
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn el_ops(&self) -> &[Op] {
        &self.el_ops
    }

    pub fn prev_l2_block_id(&self) -> &L2BlockId {
        &self.prev_l2_block_id
    }
}

/// Operation the CL pushes into the EL to perform as part of the block it's
/// producing.
#[derive(Clone, Debug)]
pub enum Op {
    /// Deposit some amount.
    Deposit(ELDepositData),
}

#[derive(Clone, Debug)]
pub struct ELDepositData {
    /// Amount in L1 native asset.  For Bitcoin this is sats.
    amt: u64,

    /// Dest addr encoded in a portable format, assumed to be valid but must be
    /// checked by EL before committing to building block.
    dest_addr: Vec<u8>,
}

impl ELDepositData {
    pub fn new(amt: u64, dest_addr: Vec<u8>) -> Self {
        Self { amt, dest_addr }
    }

    pub fn amt(&self) -> u64 {
        self.amt
    }

    pub fn dest_addr(&self) -> &[u8] {
        &self.dest_addr
    }
}

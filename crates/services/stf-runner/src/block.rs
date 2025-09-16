use sha2::{Digest, Sha256};
use strata_asm_common::AsmLogEntry;
use strata_primitives::{
    buf::{Buf32, Buf64},
    params::RollupParams,
};

use crate::account::{AccountId, SnarkAccountUpdate};

/// Represents a complete block in the Orchestration Layer (OL) chain
#[derive(Debug, Clone)]
pub struct OLBlock {
    signed_header: SignedOLBlockHeader,
    body: OLBlockBody,
}

/// A block header with a cryptographic signature
#[derive(Debug, Clone)]
pub struct SignedOLBlockHeader {
    header: OLBlockHeader,
    signature: Buf64,
}

/// The header portion of an OL block containing metadata
#[derive(Debug, Clone)]
pub struct OLBlockHeader {
    timestamp: u64,
    slot: Slot,
    epoch: Epoch,
    parent_blockid: OLBlockId,
    body_root: Buf32,
    state_root: Buf32,
}

type OLBlockId = Buf32; // TODO: change this later
type Slot = u64;
type Epoch = u64;

/// The body portion of an OL block containing the actual data
#[derive(Debug, Clone)]
pub struct OLBlockBody {
    logs: Vec<OLLog>,
    txs: Option<Vec<Transaction>>,
    l1update: Option<L1Update>,
}

/// Represents a single transaction within a block
// TODO: rename to OLTransaction?
#[derive(Debug, Clone)]
pub struct Transaction {
    type_id: u16, // maybe this is not needed here since we use enum for payload? This seems to be
    // relevant during serializing?
    payload: TransactionPayload,
    extra: TransactionExtra,
}

#[derive(Debug, Clone)]
pub enum TransactionPayload {
    GenericAccountMessage {
        target: AccountId,
        payload: Vec<u8>,
    },
    SnarkAccountUpdate {
        target: AccountId, // is the transaction supposed to update the state of this target? looks
        // like it
        update: SnarkAccountUpdate,
    },
}

/// Additional metadata for a transaction
#[derive(Debug, Clone)]
pub struct TransactionExtra {
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
}

/// Represents an update from Layer 1 blockchain
#[derive(Debug, Clone)]
pub struct L1Update {
    /// The state root before applying updates from L1
    inner_state_root: Buf32,
    /// L1 height the manifests are read upto
    new_l1_height: u32,
    /// Manifests from last l1_height to the new_l1_height
    manifests: Vec<AsmManifest>,
}

/// A manifest containing ASM (Abstract State Machine) data
#[derive(Debug, Clone)]
pub struct AsmManifest {
    blockid: Buf32,
    logs: Vec<AsmLogEntry>,
}
/// A log entry for an account operation
#[derive(Debug, Clone)]
pub struct OLLog {
    account_serial: u32,
    account_id: AccountId,
    payload: Vec<u8>, // TODO: make this typed, serialization can be done at the edges
}

impl OLBlock {
    pub fn new(signed_header: SignedOLBlockHeader, body: OLBlockBody) -> Self {
        Self {
            signed_header,
            body,
        }
    }

    pub fn signed_header(&self) -> &SignedOLBlockHeader {
        &self.signed_header
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }

    pub fn validate_block_header(
        &self,
        _params: &RollupParams,
        prev_header: &OLBlockHeader,
    ) -> Result<(), String> {
        let current_header = self.signed_header.header();

        if current_header.slot() > 0 && current_header.slot() != prev_header.slot() + 1 {
            return Err(format!("Invalid block slot {}", current_header.slot()));
        }
        if current_header.slot() > 0
            && *current_header.parent_blockid() != prev_header.compute_header_root()
        {
            return Err("Invalid parent block ID".to_string());
        }

        // Check epoch progression - epoch should not decrease
        if current_header.epoch() != 0 && current_header.epoch() < prev_header.epoch() {
            return Err(format!(
                "Epoch regression: current {} < previous {}",
                current_header.epoch, prev_header.epoch
            ));
        }

        // Check timestamp progression - should not go backwards.
        // FIXME: might need to use some threshold like bitcoin.
        if current_header.timestamp < prev_header.timestamp {
            return Err(format!(
                "Timestamp regression: current {} < previous {}",
                current_header.timestamp, prev_header.timestamp
            ));
        }

        // Basic sanity checks
        if current_header.body_root == Buf32::zero() {
            return Err("Invalid body root (zero hash)".to_string());
        }

        Ok(())
    }
}

impl SignedOLBlockHeader {
    pub fn new(header: OLBlockHeader, signature: Buf64) -> Self {
        Self { header, signature }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

impl OLBlockHeader {
    pub fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_blockid: OLBlockId,
        body_root: Buf32,
        state_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blockid,
            body_root,
            state_root,
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn parent_blockid(&self) -> &OLBlockId {
        &self.parent_blockid
    }

    pub fn body_root(&self) -> &Buf32 {
        &self.body_root
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }

    pub fn compute_header_root(&self) -> Buf32 {
        let mut hasher = Sha256::new();

        // Hash all header fields in a deterministic order
        hasher.update(self.timestamp.to_be_bytes());
        hasher.update(self.slot.to_be_bytes());
        hasher.update(self.epoch.to_be_bytes());
        hasher.update(self.parent_blockid.as_ref());
        hasher.update(self.body_root.as_ref());
        hasher.update(self.state_root.as_ref());

        Buf32::new(hasher.finalize().into())
    }
}

impl OLBlockBody {
    pub fn new(
        logs: Vec<OLLog>,
        txs: Option<Vec<Transaction>>,
        l1update: Option<L1Update>,
    ) -> Self {
        Self {
            logs,
            txs,
            l1update,
        }
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub fn txs(&self) -> &Option<Vec<Transaction>> {
        &self.txs
    }

    pub fn l1update(&self) -> &Option<L1Update> {
        &self.l1update
    }
}

impl Transaction {
    pub fn new(type_id: u16, payload: TransactionPayload, extra: TransactionExtra) -> Self {
        Self {
            type_id,
            payload,
            extra,
        }
    }

    pub fn type_id(&self) -> u16 {
        self.type_id
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn extra(&self) -> &TransactionExtra {
        &self.extra
    }

    /// The account id this transaction belongs to. Maybe we should also store sequencer pubkey
    /// along with vk? and then we can have transactions to update the pubkey if sequencer needs to
    /// rotate. Just a thought.
    pub fn account_id(&self) -> AccountId {
        match self.payload() {
            TransactionPayload::SnarkAccountUpdate { target, .. } => *target,
            // FIXME: this is probably not correct for Generic Account Message
            TransactionPayload::GenericAccountMessage { target, .. } => *target,
        }
    }
}

impl TransactionExtra {
    pub fn new(min_slot: Option<Slot>, max_slot: Option<Slot>) -> Self {
        Self { min_slot, max_slot }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        self.min_slot
    }

    pub fn max_slot(&self) -> Option<Slot> {
        self.max_slot
    }
}

impl L1Update {
    pub fn new(inner_state_root: Buf32, new_l1_height: u32, manifests: Vec<AsmManifest>) -> Self {
        Self {
            inner_state_root,
            new_l1_height,
            manifests,
        }
    }

    pub fn inner_state_root(&self) -> &Buf32 {
        &self.inner_state_root
    }

    pub fn new_l1_height(&self) -> u32 {
        self.new_l1_height
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

impl AsmManifest {
    pub fn new(blockid: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self { blockid, logs }
    }

    pub fn blockid(&self) -> &Buf32 {
        &self.blockid
    }

    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }
}

impl OLLog {
    pub fn new(account_serial: u32, account_id: AccountId, payload: Vec<u8>) -> Self {
        Self {
            account_serial,
            account_id,
            payload,
        }
    }

    pub fn account_serial(&self) -> u32 {
        self.account_serial
    }

    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

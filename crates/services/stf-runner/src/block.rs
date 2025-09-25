use sha2::{Digest, Sha256};
use strata_asm_common::AsmLogEntry;
use strata_chainexec::BlockExecutionOutput;
use strata_primitives::{
    block_credential::CredRule,
    buf::{Buf32, Buf64},
    crypto::verify_schnorr_sig,
    params::RollupParams,
};

use crate::{
    account::{AccountId, SnarkAccountUpdate},
    state::OLState,
    stf::{StfError, StfResult},
};

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
    logs_root: Buf32,
    body_root: Buf32,
    state_root: Buf32,
}

type OLBlockId = Buf32; // TODO: change this later
type Slot = u64;
type Epoch = u64;

/// The body portion of an OL block containing the actual data
#[derive(Debug, Clone)]
pub struct OLBlockBody {
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
    unsealed_state_root: Buf32,
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

    /// Check the continuity of block header
    pub fn pre_exec_validate(
        &self,
        params: &RollupParams,
        prev_header: &OLBlockHeader,
    ) -> Result<(), String> {
        self.validate_block_signature(params)?;

        let cur_header = self.signed_header.header();

        if cur_header.slot() > 0 {
            if cur_header.slot() != prev_header.slot() + 1 {
                return Err(format!("Invalid block slot {}", cur_header.slot()));
            }
            if *cur_header.parent_blockid() != prev_header.compute_header_root() {
                return Err("Invalid parent block ID".to_string());
            }
        }

        // Check epoch progression - epoch should not decrease and increase only by 1 at max
        let epoch_diff = cur_header.epoch() as i64 - prev_header.epoch() as i64;
        let valid_increment = epoch_diff == 0 || epoch_diff == 1;
        if cur_header.epoch() != 0 && !valid_increment {
            return Err(format!(
                "Epoch regression: current {} < previous {}",
                cur_header.epoch, prev_header.epoch
            ));
        }

        // Check timestamp progression - should not go backwards.
        // FIXME: might need to use some threshold like bitcoin.
        if cur_header.timestamp < prev_header.timestamp {
            return Err(format!(
                "Timestamp regression: current {} < previous {}",
                cur_header.timestamp, prev_header.timestamp
            ));
        }

        // validate body root
        let exp_root = *self.signed_header().header().body_root();
        let body_root = self.body().compute_root();
        if exp_root != body_root {
            return Err(format!(
                "Mismatched body root: expected {exp_root:}, got: {body_root}"
            ));
        }

        Ok(())
    }

    pub fn validate_block_signature(&self, params: &RollupParams) -> Result<(), String> {
        let seq_pubkey = match params.cred_rule {
            CredRule::SchnorrKey(key) => key,
            CredRule::Unchecked => return Ok(()),
        };
        let digest = self.signed_header().header().compute_header_root();

        if !verify_schnorr_sig(self.signed_header().signature(), &digest, &seq_pubkey) {
            return Err("Invalid block signature".to_string());
        }
        Ok(())
    }

    pub(crate) fn post_exec_validate(
        &self,
        out: &BlockExecutionOutput<OLState, OLLog>,
    ) -> StfResult<()> {
        // Validate state root
        let exp_root = *self.signed_header().header().state_root();
        let state_root = out.computed_state_root();
        if *state_root != exp_root {
            return Err(StfError::mismatched_state_root(exp_root, *state_root));
        };

        // validate logs root
        let exp_root = *self.signed_header().header().logs_root();
        let logs_root = OLLog::compute_root(out.logs());
        if logs_root != exp_root {
            return Err(StfError::mismatched_logs_root(exp_root, logs_root));
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
        logs_root: Buf32,
        body_root: Buf32,
        state_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blockid,
            logs_root,
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

    pub fn logs_root(&self) -> &Buf32 {
        &self.logs_root
    }

    pub fn body_root(&self) -> &Buf32 {
        &self.body_root
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }

    // NOTE: this will possibly be redundant once we have SSZ
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
    pub fn new(txs: Option<Vec<Transaction>>, l1update: Option<L1Update>) -> Self {
        Self { txs, l1update }
    }

    pub fn txs(&self) -> &Option<Vec<Transaction>> {
        &self.txs
    }

    pub fn l1update(&self) -> &Option<L1Update> {
        &self.l1update
    }

    // NOTE: this will be redundant after ssz
    pub fn compute_root(&self) -> Buf32 {
        let mut hasher = Sha256::new();
        if let Some(txs) = self.txs() {
            for tx in txs {
                hasher.update(tx.type_id().to_be_bytes());
                match &tx.payload {
                    TransactionPayload::GenericAccountMessage { target, payload } => {
                        hasher.update(target.as_slice());
                        hasher.update(payload);
                    }
                    TransactionPayload::SnarkAccountUpdate { target, update } => {
                        hasher.update(target.as_slice());
                        hasher.update(&update.witness);
                        hasher.update(update.data.seq_no.to_be_bytes());
                        // TODO: other fields, maybe wait for ssz?
                        todo!()
                    }
                }
            }
        }
        Buf32::new(hasher.finalize().into())
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

    /// The account id this transaction is on behalf of. `target` is confusing.
    /// Maybe we could also store sequencer pubkey
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
            unsealed_state_root: inner_state_root,
            new_l1_height,
            manifests,
        }
    }

    pub fn inner_state_root(&self) -> &Buf32 {
        &self.unsealed_state_root
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
    pub fn new(account_serial: u32, payload: Vec<u8>) -> Self {
        Self {
            account_serial,
            payload,
        }
    }

    pub fn account_serial(&self) -> u32 {
        self.account_serial
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    // NOTE: This will also be redundant after SSZ
    pub(crate) fn compute_root(logs: &[Self]) -> Buf32 {
        let mut hasher = Sha256::new();
        for log in logs {
            hasher.update(log.account_serial().to_be_bytes());
            hasher.update(log.payload());
        }
        Buf32::new(hasher.finalize().into())
    }
}

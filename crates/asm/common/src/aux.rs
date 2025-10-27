use std::collections::{BTreeMap, HashSet};

use bitcoin::Txid;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_btc_types::L1Tx;
use strata_identifiers::L1BlockId;
use strata_l1_txfmt::SubprotocolId;
use strata_mmr::MerkleProof;

use crate::{AsmLogEntry, L1TxIndex};

/// Table mapping subprotocol IDs to their corresponding auxiliary payloads.
pub type AuxDataTable = BTreeMap<SubprotocolId, AuxInput>;

/// Table mapping subprotocol IDs to their corresponding auxiliary requests.
pub type AuxRequestTable = BTreeMap<SubprotocolId, AuxRequests>;

/// Compact representation of an MMR proof for an ASM log leaf.
pub type LogMmrProof = MerkleProof<[u8; 32]>;

/// Request for ASM logs emitted across a contiguous range of L1 blocks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AsmLogRequest {
    /// The L1 transaction index that is requesting this auxiliary data.
    pub requester_tx_index: L1TxIndex,
    /// Start of the block range (inclusive).
    pub start_block: u64,
    /// End of the block range (inclusive).
    pub end_block: u64,
}

/// Request for full L1 transaction data.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct L1TxRequest {
    /// The L1 transaction index that is requesting this auxiliary data.
    pub requester_tx_index: L1TxIndex,
    /// The transaction ID to fetch.
    pub txid: Txid,
}

/// Container for auxiliary data requests emitted during preprocessing.
///
/// This is the request-side counterpart to [`AuxData`], containing all auxiliary
/// inputs that subprotocols need before processing transactions.
/// /// Duplicate requests are automatically ignored due to the underlying `HashSet`.
#[derive(Debug, Default, Clone)]
pub struct AuxRequests {
    /// Requests for ASM logs from historical blocks.
    pub asm_log_requests: HashSet<AsmLogRequest>,
    /// Requests for L1 transaction data.
    pub l1_tx_requests: HashSet<L1TxRequest>,
}

impl AuxRequests {
    /// Creates a new empty auxiliary requests container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if there are no auxiliary requests.
    pub fn is_empty(&self) -> bool {
        self.asm_log_requests.is_empty() && self.l1_tx_requests.is_empty()
    }

    /// Adds an ASM log request.
    pub fn request_asm_logs(&mut self, tx_index: L1TxIndex, start_block: u64, end_block: u64) {
        self.asm_log_requests.insert(AsmLogRequest {
            requester_tx_index: tx_index,
            start_block,
            end_block,
        });
    }

    /// Adds an L1 transaction request.
    pub fn request_l1_tx(&mut self, tx_index: L1TxIndex, txid: Txid) {
        self.l1_tx_requests.insert(L1TxRequest {
            requester_tx_index: tx_index,
            txid,
        });
    }
}

/// Per-block set of historical logs together with an MMR inclusion proof.
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct AsmLogsOracleData {
    pub block_hash: L1BlockId,
    pub logs: Vec<AsmLogEntry>,
    pub proof: LogMmrProof,
}

/// L1 transaction auxiliary data response containing raw Bitcoin transaction data
/// and merkle proofs for verification.
// TODO: ensure the transaction is valid and part of the canonical chain, verify:
// 1. Chain inclusion: The block containing this transaction is part of the
///    canonical L1 chain (verified via MMR proof of the block's log leaf)
// 2. Transaction inclusion: The transaction exists in the claimed block
///    (verified via Merkle inclusion proof against the block's tx merkle root)
// 3. Transaction validity: The transaction data deserializes to a valid Bitcoin transaction
//    structure
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct L1TxOracleData {
    pub tx: L1Tx,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct AuxData {
    pub asm_logs_oracle: Vec<AsmLogsOracleData>,
    pub l1_txs_oracle: L1TxOracleData,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct AuxInput {
    pub data: BTreeMap<L1TxIndex, AuxData>,
}

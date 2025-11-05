use std::collections::BTreeMap;

use bitcoin::Txid;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_identifiers::{Buf32, L1BlockId};
use strata_l1_txfmt::SubprotocolId;
use strata_merkle::MerkleProof;

use crate::{AsmLogEntry, AsmManifest, L1TxIndex};

/// Table mapping subprotocol IDs to their corresponding auxiliary payloads.
pub type AuxDataTable = BTreeMap<SubprotocolId, AuxResponses>;

/// Table mapping subprotocol IDs to their corresponding auxiliary requests.
pub type AuxRequestTable = BTreeMap<SubprotocolId, AuxRequests>;

/// Compact representation of an MMR proof for an ASM log leaf.
pub type LogMmrProof = MerkleProof<[u8; 32]>;

/// Query for ASM logs emitted across a contiguous range of L1 blocks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AsmLogQuery {
    pub requester_tx_index: L1TxIndex,
    pub start_block: u64,
    pub end_block: u64,
}

/// Query for fetching an L1 transaction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct L1TxQuery {
    pub requester_tx_index: L1TxIndex,
    pub txid: Txid,
}
/// Container for auxiliary data requests emitted during preprocessing.
///
/// This is the request-side counterpart to [`AuxResponses`], containing all auxiliary
/// inputs that subprotocols need before processing transactions. Duplicate requests
/// are automatically ignored based on their content.
#[derive(Debug, Clone, Default)]
pub struct AuxRequests {
    asm_log_queries: Vec<AsmLogQuery>,
    l1_tx_queries: Vec<L1TxQuery>,
}

impl AuxRequests {
    /// Creates a new empty auxiliary requests container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if there are no auxiliary requests.
    pub fn is_empty(&self) -> bool {
        self.asm_log_queries.is_empty() && self.l1_tx_queries.is_empty()
    }

    /// Adds an ASM log request.
    pub fn request_asm_logs(
        &mut self,
        requester_tx_index: L1TxIndex,
        start_block: u64,
        end_block: u64,
    ) {
        let query = AsmLogQuery {
            requester_tx_index,
            start_block,
            end_block,
        };
        self.asm_log_queries.push(query);
    }

    /// Adds an L1 transaction request.
    pub fn request_l1_tx(&mut self, requester_tx_index: L1TxIndex, txid: Txid) {
        let query = L1TxQuery {
            requester_tx_index,
            txid,
        };
        self.l1_tx_queries.push(query);
    }
}

/// Auxiliary responses for a single subprotocol.
#[derive(Debug, Clone, Default, BorshDeserialize, BorshSerialize)]
pub struct AuxResponses {
    pub data: BTreeMap<L1TxIndex, AuxResponseBatch>,
}

/// Raw auxiliary responses (claims + proofs) keyed by request identifier.
#[derive(Debug, Clone, Default, BorshDeserialize, BorshSerialize)]
pub struct AuxResponseBatch {
    pub asm_logs: Vec<AsmLogClaim>,
    pub l1_txs: Vec<L1TxClaim>,
}

/// ASM log claim (oracle data + proof).
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct AsmLogClaim {
    pub claim: AsmManifest,
    pub proof: LogMmrProof,
}

/// Proof bundle for an L1 transaction claim.
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum L1TxProofBundle {
    TxidOnly { expected_txid: Buf32 },
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct L1TxClaim {
    // TODO: For now we use the raw L1Tx bytes as the claim, but in the future we should
    // switch to using the btc_types::L1Tx type after protocol ops are removed there.
    pub claim: Vec<u8>,
    pub proof: L1TxProofBundle,
}

/// Verified auxiliary inputs grouped by requesting transaction.
#[derive(Debug, Clone, Default)]
pub struct VerifiedAuxInput {
    pub data: BTreeMap<L1TxIndex, VerifiedAuxData>,
}

impl VerifiedAuxInput {
    pub fn get(&self, tx_index: L1TxIndex) -> Option<&VerifiedAuxData> {
        self.data.get(&tx_index)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&L1TxIndex, &VerifiedAuxData)> {
        self.data.iter()
    }
}

/// Verified auxiliary data for a specific requesting transaction.
#[derive(Debug, Clone, Default)]
pub struct VerifiedAuxData {
    pub asm_logs: Vec<AsmLogOracle>,
    pub l1_txs: Vec<L1TxOracle>,
}

impl VerifiedAuxData {
    pub fn is_empty(&self) -> bool {
        self.asm_logs.is_empty() && self.l1_txs.is_empty()
    }
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct AsmLogOracle {
    pub block_hash: L1BlockId,
    pub logs: Vec<AsmLogEntry>,
}

/// Verified L1 transaction response.
#[derive(Debug, Clone)]
pub struct L1TxOracle {
    pub tx: Vec<u8>,
}

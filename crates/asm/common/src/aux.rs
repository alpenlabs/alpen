use std::{any::Any, collections::BTreeMap};

use bitcoin::{BlockHash, Txid, hashes::Hash};
use borsh::{
    BorshDeserialize, BorshSerialize,
    io::{Read, Write},
};
use strata_l1_txfmt::SubprotocolId;
use strata_mmr::MerkleProof;

use crate::{AsmLogEntry, L1TxIndex};

/// Table mapping subprotocol IDs to their corresponding auxiliary payloads.
pub type AuxDataTable = BTreeMap<SubprotocolId, AuxInput>;

/// Compact representation of an MMR proof for an ASM log leaf.
pub type LogMmrProof = MerkleProof<[u8; 32]>;

/// Supported auxiliary queries that subprotocols can register during preprocessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuxRequestSpec {
    /// Request logs emitted across a contiguous range of L1 blocks (inclusive).
    AsmLogQueries { start_block: u64, end_block: u64 },
    /// Request the full deposit request transaction referenced by the deposit.
    DepositRequestTx { txid: Txid },
}

/// Responsible for recording auxiliary requests emitted during preprocessing.
pub trait AuxRequestCollector: Any {
    /// Records that the transaction at `tx_index` requires auxiliary data described by `request`.
    fn request_aux_input(&mut self, tx_index: L1TxIndex, request: AuxRequestSpec);

    /// Exposes the collector as a `&mut dyn Any` for downcasting.
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

/// Per-block set of historical logs together with an MMR inclusion proof.
#[derive(Debug, Clone)]
pub struct BlockLogsOracleData {
    pub block_hash: BlockHash,
    pub logs: Vec<AsmLogEntry>,
    pub proof: LogMmrProof,
}

impl BorshSerialize for BlockLogsOracleData {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.block_hash.to_byte_array().serialize(writer)?;
        self.logs.serialize(writer)?;
        let cohashes: Vec<[u8; 32]> = self.proof.cohashes().to_vec();
        cohashes.serialize(writer)?;
        self.proof.index().serialize(writer)?;
        Ok(())
    }
}

impl BorshDeserialize for BlockLogsOracleData {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let block_hash_bytes = <[u8; 32]>::deserialize_reader(reader)?;
        let logs = Vec::<AsmLogEntry>::deserialize_reader(reader)?;
        let cohashes = Vec::<[u8; 32]>::deserialize_reader(reader)?;
        let index = u64::deserialize_reader(reader)?;
        let proof = LogMmrProof::from_cohashes(cohashes, index);
        Ok(Self {
            block_hash: BlockHash::from_byte_array(block_hash_bytes),
            logs,
            proof,
        })
    }
}

/// DRT transaction auxiliary data response.
// TODO: Update field type as needed.
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct DrtTxOracleData {
    pub drt_tx: Vec<u8>,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct AuxData {
    pub asm_log_oracles: Vec<BlockLogsOracleData>,
    pub drt_tx_oracles: Vec<DrtTxOracleData>,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct AuxInput {
    pub data: BTreeMap<L1TxIndex, AuxData>,
}

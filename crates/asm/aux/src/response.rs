use bitcoin::{BlockHash, hashes::Hash as HashTrait};
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLogEntry;
use strata_mmr::MerkleProof;

/// Compact representation of an MMR proof for an ASM log leaf.
pub type LogMmrProof = MerkleProof<[u8; 32]>;

/// Per-block set of historical logs together with an MMR inclusion proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoricalLogSegment {
    pub block_hash: BlockHash,
    pub logs: Vec<AsmLogEntry>,
    pub proof: LogMmrProof,
}

impl HistoricalLogSegment {
    pub fn new(block_hash: BlockHash, logs: Vec<AsmLogEntry>, proof: LogMmrProof) -> Self {
        Self {
            block_hash,
            logs,
            proof,
        }
    }
}

impl BorshSerialize for HistoricalLogSegment {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        let hash_bytes = self.block_hash.to_byte_array();
        hash_bytes.serialize(writer)?;
        self.logs.serialize(writer)?;
        let index = self.proof.index();
        index.serialize(writer)?;
        let cohashes: Vec<[u8; 32]> = self.proof.cohashes().to_vec();
        cohashes.serialize(writer)?;
        Ok(())
    }
}

impl BorshDeserialize for HistoricalLogSegment {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let hash_bytes = <[u8; 32]>::deserialize_reader(reader)?;
        let block_hash = BlockHash::from_byte_array(hash_bytes);
        let logs = Vec::<AsmLogEntry>::deserialize_reader(reader)?;
        let index = u64::deserialize_reader(reader)?;
        let cohashes = Vec::<[u8; 32]>::deserialize_reader(reader)?;
        let proof = LogMmrProof::from_cohashes(cohashes, index);
        Ok(Self {
            block_hash,
            logs,
            proof,
        })
    }
}

/// Typed auxiliary data returned to the STF for a specific transaction.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AuxResponseEnvelope {
    /// Historical ASM logs together with inclusion proofs for a set of blocks.
    HistoricalLogs { segments: Vec<HistoricalLogSegment> },
    /// Historical ASM logs covering a range of L1 blocks (inclusive).
    HistoricalLogsRange {
        start_block: u64,
        end_block: u64,
        segments: Vec<HistoricalLogSegment>,
    },
    /// TODO: update this variant when implementing drt validation with aux.
    DepositRequestTx { raw_tx: Vec<u8> },
}

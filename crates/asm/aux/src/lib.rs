use borsh::{BorshDeserialize, BorshSerialize};
use strata_acct_types::MerkleProof;
use strata_asm_logs::AuxLog;
use strata_msg_fmt::TypeId;

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum AuxRequestSpec {
    /// Query for historical auxiliary data at a specific block number.
    /// This is useful for fetching a specific aux data from a single block.
    HistoricalAuxQuery {
        /// The ID of log type to query.
        log_type_id: TypeId,
        /// The block number at which to query the auxiliary input.
        block_number: u64,
    },

    /// Query for historical auxiliary data within a block range.
    /// This is useful for fetching multiple aux data in a single request from multiple blocks.
    HistoricalAuxRangeQuery {
        /// The ID of log type to query.
        log_type_id: Vec<TypeId>,
        /// The starting block number (inclusive) of the range to query.
        start_block: u64,
        /// The ending block number (inclusive) of the range to query.
        end_block: u64,
    },
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HistoricalAuxResponse {
    pub log: AuxLog,
    pub proof: SerializableMerkleProof,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HistoricalAuxRangeResponse {
    pub logs: Vec<AuxLog>,
    pub proofs: Vec<SerializableMerkleProof>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "TODO: should we refactor to avoid large enum variants"
)]
pub enum AuxResponseSpec {
    Single(HistoricalAuxResponse),
    Range(HistoricalAuxRangeResponse),
}



#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SerializableMerkleProof {
    pub index: u64,
    pub cohashes: Vec<[u8; 32]>,
}

impl From<&MerkleProof> for SerializableMerkleProof {
    fn from(proof: &MerkleProof) -> Self {
        Self {
            index: proof.index(),
            cohashes: proof.cohashes().to_vec(),
        }
    }
}

impl SerializableMerkleProof {
    pub fn into_merkle_proof(self) -> MerkleProof {
        MerkleProof::from_cohashes(self.cohashes, self.index)
    }
}

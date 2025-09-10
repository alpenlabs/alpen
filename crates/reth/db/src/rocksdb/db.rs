use std::sync::Arc;

use alpen_reth_statediff::BlockStateDiff;
use revm_primitives::alloy_primitives::B256;
use rockbound::{SchemaDBOperations, SchemaDBOperationsExt};
use strata_proofimpl_evm_ee_stf::primitives::EvmBlockStfInput;

use super::schema::{BlockHashByNumber, BlockStateDiffSchema, BlockWitnessSchema};
use crate::{
    errors::DbError, DbResult, StateDiffProvider, StateDiffStore, WitnessProvider, WitnessStore,
};

#[derive(Debug)]
pub struct WitnessDB<DB> {
    db: Arc<DB>,
}

// FIXME: cannot derive Clone with a generic parameter that does not implement Clone
// @see https://github.com/rust-lang/rust/issues/26925
impl<DB> Clone for WitnessDB<DB> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}

impl<DB> WitnessDB<DB> {
    pub fn new(db: Arc<DB>) -> anyhow::Result<Self> {
        Ok(Self { db })
    }
}

impl<DB: SchemaDBOperations> WitnessProvider for WitnessDB<DB> {
    fn get_block_witness(&self, block_hash: B256) -> DbResult<Option<EvmBlockStfInput>> {
        let raw = self.db.get::<BlockWitnessSchema>(&block_hash)?;

        let parsed: Option<EvmBlockStfInput> = raw
            .map(|bytes| bincode::deserialize(&bytes))
            .transpose()
            .map_err(|err| DbError::CodecError(err.to_string()))?;

        Ok(parsed)
    }

    fn get_block_witness_raw(&self, block_hash: B256) -> DbResult<Option<Vec<u8>>> {
        Ok(self.db.get::<BlockWitnessSchema>(&block_hash)?)
    }
}

impl<DB: SchemaDBOperations> WitnessStore for WitnessDB<DB> {
    fn put_block_witness(
        &self,
        block_hash: B256,
        witness: &EvmBlockStfInput,
    ) -> crate::DbResult<()> {
        let serialized =
            bincode::serialize(witness).map_err(|err| DbError::Other(err.to_string()))?;
        Ok(self
            .db
            .put::<BlockWitnessSchema>(&block_hash, &serialized)?)
    }

    fn del_block_witness(&self, block_hash: B256) -> DbResult<()> {
        Ok(self.db.delete::<BlockWitnessSchema>(&block_hash)?)
    }
}

impl<DB: SchemaDBOperations> StateDiffProvider for WitnessDB<DB> {
    fn get_state_diff_by_hash(&self, block_hash: B256) -> DbResult<Option<BlockStateDiff>> {
        let raw = self.db.get::<BlockStateDiffSchema>(&block_hash)?;

        let parsed: Option<BlockStateDiff> = raw
            .map(|bytes| bincode::deserialize(&bytes))
            .transpose()
            .map_err(|err| DbError::CodecError(err.to_string()))?;

        Ok(parsed)
    }

    fn get_state_diff_by_number(&self, block_number: u64) -> DbResult<Option<BlockStateDiff>> {
        let block_hash = self.db.get::<BlockHashByNumber>(&block_number)?;
        if block_hash.is_none() {
            return DbResult::Ok(None);
        }

        self.get_state_diff_by_hash(B256::from_slice(&block_hash.unwrap()))
    }
}

impl<DB: SchemaDBOperations> StateDiffStore for WitnessDB<DB> {
    fn put_state_diff(
        &self,
        block_hash: B256,
        block_number: u64,
        witness: &BlockStateDiff,
    ) -> crate::DbResult<()> {
        self.db
            .put::<BlockHashByNumber>(&block_number, &block_hash.to_vec())?;

        let serialized =
            bincode::serialize(witness).map_err(|err| DbError::Other(err.to_string()))?;
        Ok(self
            .db
            .put::<BlockStateDiffSchema>(&block_hash, &serialized)?)
    }

    fn del_state_diff(&self, block_hash: B256) -> DbResult<()> {
        Ok(self.db.delete::<BlockStateDiffSchema>(&block_hash)?)
    }
}

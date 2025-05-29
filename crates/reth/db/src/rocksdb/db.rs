use std::sync::Arc;

use alpen_reth_statediff::BlockStateDiff;
use revm_primitives::alloy_primitives::B256;
use rockbound::{SchemaDBOperations, SchemaDBOperationsExt};
use strata_proofimpl_evm_ee_stf::EvmBlockStfInput;

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
    pub const fn new(db: Arc<DB>) -> Self {
        Self { db }
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

#[cfg(test)]
mod tests {

    use alpen_reth_statediff::account::{Account, AccountChanges};
    use revm_primitives::{
        alloy_primitives::{address, map::HashMap},
        fixed_bytes, FixedBytes, HashSet,
    };
    use rockbound::SchemaDBOperations;
    use serde::Deserialize;
    use strata_proofimpl_evm_ee_stf::{EvmBlockStfInput, EvmBlockStfOutput};
    use tempfile::TempDir;

    use super::*;

    const BLOCK_HASH_ONE: FixedBytes<32> =
        fixed_bytes!("000000000000000000000000f529c70db0800449ebd81fbc6e4221523a989f05");
    const BLOCK_HASH_TWO: FixedBytes<32> =
        fixed_bytes!("0000000000000000000000000a743ba7304efcc9e384ece9be7631e2470e401e");

    fn get_rocksdb_tmp_instance() -> anyhow::Result<impl SchemaDBOperations> {
        let dbname = crate::rocksdb::ROCKSDB_NAME;
        let cfs = crate::rocksdb::STORE_COLUMN_FAMILIES;
        let mut opts = rockbound::rocksdb::Options::default();
        opts.create_missing_column_families(true);
        opts.create_if_missing(true);

        let temp_dir = TempDir::new().expect("failed to create temp dir");

        let rbdb = rockbound::DB::open(
            temp_dir.into_path(),
            dbname,
            cfs.iter().map(|s| s.to_string()),
            &opts,
        )?;

        Ok(rbdb)
    }

    #[derive(Deserialize)]
    struct TestData {
        witness: EvmBlockStfInput,
        params: EvmBlockStfOutput,
    }

    fn get_mock_data() -> TestData {
        let json_content = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_data/witness_params.json"),
        )
        .expect("Failed to read the blob data file");

        serde_json::from_str(&json_content).expect("Valid json")
    }

    fn test_state_diff() -> BlockStateDiff {
        let mut test_diff = BlockStateDiff {
            state: HashMap::default(),
            contracts: HashSet::default(),
        };

        test_diff.state.insert(
            address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045"),
            AccountChanges::new(None, Some(Account::default()), HashMap::default()),
        );

        test_diff
    }

    fn setup_db() -> WitnessDB<impl SchemaDBOperations> {
        let db = get_rocksdb_tmp_instance().unwrap();
        WitnessDB::new(Arc::new(db))
    }

    #[test]
    fn set_and_get_witness_data() {
        let db = setup_db();

        let test_data = get_mock_data();
        let block_hash = test_data.params.new_blockhash;

        db.put_block_witness(block_hash, &test_data.witness)
            .expect("failed to put witness data");

        // assert block was stored
        let received_witness = db
            .get_block_witness(block_hash)
            .expect("failed to retrieve witness data")
            .unwrap();

        assert_eq!(received_witness, test_data.witness);
    }

    #[test]
    fn del_and_get_block_data() {
        let db = setup_db();
        let test_data = get_mock_data();
        let block_hash = test_data.params.new_blockhash;

        // assert block is not present in the db
        let received_witness = db.get_block_witness(block_hash);
        assert!(matches!(received_witness, Ok(None)));

        // deleting non existing block is ok
        let res = db.del_block_witness(block_hash);
        assert!(matches!(res, Ok(())));

        db.put_block_witness(block_hash, &test_data.witness)
            .expect("failed to put witness data");
        // assert block is present in the db
        let received_witness = db.get_block_witness(block_hash);
        assert!(matches!(
            received_witness,
            Ok(Some(EvmBlockStfInput { .. }))
        ));

        // deleting existing block is ok
        let res = db.del_block_witness(block_hash);
        assert!(matches!(res, Ok(())));

        // assert block is deleted from the db
        let received_witness = db.get_block_witness(block_hash);
        assert!(matches!(received_witness, Ok(None)));
    }

    #[test]
    fn set_and_get_state_diff_data() {
        let db = setup_db();

        let test_state_diff = test_state_diff();
        let block_hash = BLOCK_HASH_ONE;

        db.put_state_diff(block_hash, 1, &test_state_diff)
            .expect("failed to put witness data");

        // assert block was stored
        let received_state_diff = db
            .get_state_diff_by_hash(block_hash)
            .expect("failed to retrieve witness data")
            .unwrap();

        assert_eq!(received_state_diff, test_state_diff);
    }

    #[test]
    fn del_and_get_state_diff_data() {
        let db = setup_db();
        let test_state_diff = test_state_diff();
        let block_hash = BLOCK_HASH_TWO;

        // assert block is not present in the db
        let received_state_diff = db.get_state_diff_by_hash(block_hash);
        assert!(matches!(received_state_diff, Ok(None)));

        // deleting non existing block is ok
        let res = db.del_block_witness(block_hash);
        assert!(matches!(res, Ok(())));

        db.put_state_diff(block_hash, 7, &test_state_diff)
            .expect("failed to put state diff data");
        // assert block is present in the db
        let received_state_diff = db.get_state_diff_by_hash(block_hash);
        assert!(matches!(
            received_state_diff,
            Ok(Some(BlockStateDiff { .. }))
        ));

        // deleting existing block is ok
        let res = db.del_state_diff(block_hash);
        assert!(matches!(res, Ok(())));

        // assert block is deleted from the db
        let received_state_diff = db.get_state_diff_by_hash(block_hash);
        assert!(matches!(received_state_diff, Ok(None)));
    }
}

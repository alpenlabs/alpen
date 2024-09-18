use std::sync::Arc;

use express_proofimpl_evm_ee_stf::ELProofInput;
use reth_primitives::B256;
use rockbound::{SchemaDBOperations, SchemaDBOperationsExt};

use super::schema::BlockWitnessSchema;
use crate::{errors::DbError, DbResult, WitnessProvider, WitnessStore};

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
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl<DB: SchemaDBOperations> WitnessProvider for WitnessDB<DB> {
    fn get_block_witness(&self, block_hash: B256) -> DbResult<Option<ELProofInput>> {
        let raw = self.db.get::<BlockWitnessSchema>(&block_hash)?;

        let parsed: Option<ELProofInput> = raw
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
    fn put_block_witness(&self, block_hash: B256, witness: &ELProofInput) -> crate::DbResult<()> {
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

#[cfg(test)]
mod tests {
    use express_proofimpl_evm_ee_stf::{ELProofInput, ELProofPublicParams};
    use rockbound::SchemaDBOperations;
    use serde::Deserialize;
    use tempfile::TempDir;

    use super::*;

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
        witness: ELProofInput,
        params: ELProofPublicParams,
    }

    fn get_mock_data() -> TestData {
        let json_content = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_data/witness_params.json"),
        )
        .expect("Failed to read the blob data file");

        serde_json::from_str(&json_content).expect("Valid json")
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
        assert!(matches!(received_witness, Ok(Some(ELProofInput { .. }))));

        // deleting existing block is ok
        let res = db.del_block_witness(block_hash);
        assert!(matches!(res, Ok(())));

        // assert block is deleted from the db
        let received_witness = db.get_block_witness(block_hash);
        assert!(matches!(received_witness, Ok(None)));
    }
}

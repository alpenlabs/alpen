use std::sync::Arc;

use alpen_reth_statediff::BlockStateChanges;
use revm_primitives::alloy_primitives::B256;
use sled::transaction::{ConflictableTransactionError, ConflictableTransactionResult};
use strata_proofimpl_evm_ee_stf::primitives::EvmBlockStfInput;
use typed_sled::{error::Error, transaction::SledTransactional, SledDb, SledTree};

use super::schema::{BlockHashByNumber, BlockStateChangesSchema, BlockWitnessSchema};
use crate::{
    errors::DbError, DbResult, StateDiffProvider, StateDiffStore, WitnessProvider, WitnessStore,
};

#[derive(Debug)]
pub struct WitnessDB {
    witness_tree: SledTree<BlockWitnessSchema>,
    state_diff_tree: SledTree<BlockStateChangesSchema>,
    block_hash_by_number_tree: SledTree<BlockHashByNumber>,
}

impl Clone for WitnessDB {
    fn clone(&self) -> Self {
        Self {
            witness_tree: self.witness_tree.clone(),
            state_diff_tree: self.state_diff_tree.clone(),
            block_hash_by_number_tree: self.block_hash_by_number_tree.clone(),
        }
    }
}

impl WitnessDB {
    pub fn new(db: Arc<SledDb>) -> Result<Self, Error> {
        let witness_tree = db.get_tree::<BlockWitnessSchema>()?;
        let state_diff_tree = db.get_tree::<BlockStateChangesSchema>()?;
        let block_hash_by_number_tree = db.get_tree::<BlockHashByNumber>()?;

        Ok(Self {
            witness_tree,
            state_diff_tree,
            block_hash_by_number_tree,
        })
    }
}

impl WitnessProvider for WitnessDB {
    fn get_block_witness(&self, block_hash: B256) -> DbResult<Option<EvmBlockStfInput>> {
        let raw = self.witness_tree.get(&block_hash)?;

        let parsed: Option<EvmBlockStfInput> = raw
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()
            .map_err(|err| DbError::CodecError(err.to_string()))?;

        Ok(parsed)
    }

    fn get_block_witness_raw(&self, block_hash: B256) -> DbResult<Option<Vec<u8>>> {
        Ok(self.witness_tree.get(&block_hash)?)
    }
}

impl WitnessStore for WitnessDB {
    fn put_block_witness(&self, block_hash: B256, witness: &EvmBlockStfInput) -> DbResult<()> {
        let serialized =
            serde_json::to_vec(witness).map_err(|err| DbError::Other(err.to_string()))?;

        Ok(self.witness_tree.insert(&block_hash, &serialized)?)
    }

    fn del_block_witness(&self, block_hash: B256) -> DbResult<()> {
        Ok(self.witness_tree.remove(&block_hash)?)
    }
}

impl StateDiffProvider for WitnessDB {
    fn get_state_diff_by_hash(&self, block_hash: B256) -> DbResult<Option<BlockStateChanges>> {
        let raw = self.state_diff_tree.get(&block_hash)?;

        let parsed: Option<BlockStateChanges> = raw
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()
            .map_err(|err| DbError::CodecError(err.to_string()))?;

        Ok(parsed)
    }

    fn get_state_diff_by_number(&self, block_number: u64) -> DbResult<Option<BlockStateChanges>> {
        let block_hash = self
            .block_hash_by_number_tree
            .get(&block_number)
            .map_err(|err| DbError::Other(err.to_string()))?;

        if block_hash.is_none() {
            return Ok(None);
        }

        self.get_state_diff_by_hash(B256::from_slice(&block_hash.unwrap()))
    }
}

impl StateDiffStore for WitnessDB {
    fn put_state_diff(
        &self,
        block_hash: B256,
        block_number: u64,
        state_diff: &BlockStateChanges,
    ) -> DbResult<()> {
        (&self.block_hash_by_number_tree, &self.state_diff_tree)
            .transaction(|(bht, sdt)| -> ConflictableTransactionResult<(), Error> {
                bht.insert(&block_number, &block_hash.to_vec())?;
                let serialized = match serde_json::to_vec(state_diff) {
                    Ok(data) => data,
                    Err(err) => {
                        return Err(ConflictableTransactionError::Abort(
                            sled::Error::Unsupported(format!("Serialization failed: {}", err))
                                .into(),
                        ))
                    }
                };
                sdt.insert(&block_hash, &serialized)?;
                Ok(())
            })
            .map_err(|e| DbError::Other(format!("{:?}", e)))?;
        Ok(())
    }

    fn del_state_diff(&self, block_hash: B256) -> DbResult<()> {
        Ok(self.state_diff_tree.remove(&block_hash)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs::read_to_string, path::PathBuf};

    use alpen_reth_statediff::{
        AccountSnapshot, BlockAccountChange, BlockStateChanges, BlockStorageDiff,
    };
    use revm_primitives::{address, fixed_bytes, FixedBytes, KECCAK_EMPTY, U256};
    use serde::Deserialize;
    use strata_proofimpl_evm_ee_stf::primitives::{EvmBlockStfInput, EvmBlockStfOutput};
    use typed_sled::SledDb;

    use super::*;

    const BLOCK_HASH_ONE: FixedBytes<32> =
        fixed_bytes!("000000000000000000000000f529c70db0800449ebd81fbc6e4221523a989f05");
    const BLOCK_HASH_TWO: FixedBytes<32> =
        fixed_bytes!("0000000000000000000000000a743ba7304efcc9e384ece9be7631e2470e401e");

    fn get_sled_tmp_instance() -> SledDb {
        let db = sled::Config::new().temporary(true).open().unwrap();
        SledDb::new(db).unwrap()
    }

    #[derive(Deserialize)]
    struct TestData {
        witness: EvmBlockStfInput,
        params: EvmBlockStfOutput,
    }

    fn get_mock_data() -> TestData {
        let json_content = read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_data/witness_params.json"),
        )
        .expect("Failed to read the blob data file");

        serde_json::from_str(&json_content).expect("Valid json")
    }

    fn setup_db() -> WitnessDB {
        let db = Arc::new(get_sled_tmp_instance());
        WitnessDB::new(db).unwrap()
    }

    fn test_state_diff() -> BlockStateChanges {
        let mut accounts = BTreeMap::new();
        accounts.insert(
            address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045"),
            BlockAccountChange {
                original: None,
                current: Some(AccountSnapshot {
                    balance: U256::from(1000),
                    nonce: 1,
                    code_hash: KECCAK_EMPTY,
                }),
            },
        );

        let mut storage = BTreeMap::new();
        let mut slots = BlockStorageDiff::new();
        slots
            .slots
            .insert(U256::from(1), (U256::ZERO, U256::from(100)));
        storage.insert(
            address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045"),
            slots,
        );

        BlockStateChanges {
            accounts,
            storage,
            deployed_code_hashes: vec![],
        }
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

        // Check accounts and storage match
        assert_eq!(
            received_state_diff.accounts.len(),
            test_state_diff.accounts.len()
        );
        assert_eq!(
            received_state_diff.storage.len(),
            test_state_diff.storage.len()
        );
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
            Ok(Some(BlockStateChanges { .. }))
        ));

        // deleting existing block is ok
        let res = db.del_state_diff(block_hash);
        assert!(matches!(res, Ok(())));

        // assert block is deleted from the db
        let received_state_diff = db.get_state_diff_by_hash(block_hash);
        assert!(matches!(received_state_diff, Ok(None)));
    }
}

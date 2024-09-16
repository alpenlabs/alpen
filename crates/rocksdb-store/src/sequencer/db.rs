use std::sync::Arc;

use alpen_express_db::{
    errors::DbError,
    traits::{SeqDataProvider, SeqDataStore, SequencerDatabase},
    types::BlobEntry,
    DbResult,
};
use alpen_express_primitives::buf::Buf32;
use alpen_express_state::batch::BatchCommitment;
use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};

use super::schemas::{BatchCommitmentSchema, SeqBlobIdSchema, SeqBlobSchema};
use crate::DbOpsConfig;

pub struct SeqDb {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl SeqDb {
    /// Wraps an existing database handle.
    ///
    /// Assumes it was opened with column families as defined in `STORE_COLUMN_FAMILIES`.
    // FIXME Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl SeqDataStore for SeqDb {
    fn put_blob_entry(&self, blob_hash: Buf32, blob: BlobEntry) -> DbResult<()> {
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |tx| -> Result<(), DbError> {
                    // If new, increment idx
                    if tx.get::<SeqBlobSchema>(&blob_hash)?.is_none() {
                        let idx = rockbound::utils::get_last::<SeqBlobIdSchema>(tx)?
                            .map(|(x, _)| x + 1)
                            .unwrap_or(0);

                        tx.put::<SeqBlobIdSchema>(&idx, &blob_hash)?;
                    }

                    tx.put::<SeqBlobSchema>(&blob_hash, &blob)?;

                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn put_batch_commitment(
        &self,
        batchidx: u64,
        batch_commitment: alpen_express_state::batch::BatchCommitment,
    ) -> DbResult<()> {
        if self.db.get::<BatchCommitmentSchema>(&batchidx)?.is_some() {
            return Err(DbError::OverwriteBatchCommitment(batchidx));
        };
        self.db
            .put::<BatchCommitmentSchema>(&batchidx, &batch_commitment)?;
        Ok(())
    }
}

impl SeqDataProvider for SeqDb {
    fn get_blob_by_id(&self, id: Buf32) -> DbResult<Option<BlobEntry>> {
        Ok(self.db.get::<SeqBlobSchema>(&id)?)
    }

    fn get_last_blob_idx(&self) -> DbResult<Option<u64>> {
        Ok(rockbound::utils::get_last::<SeqBlobIdSchema>(&*self.db)?.map(|(x, _)| x))
    }

    fn get_blob_id(&self, blobidx: u64) -> DbResult<Option<Buf32>> {
        Ok(self.db.get::<SeqBlobIdSchema>(&blobidx)?)
    }

    fn get_batch_commitment(&self, batchidx: u64) -> DbResult<Option<BatchCommitment>> {
        Ok(self.db.get::<BatchCommitmentSchema>(&batchidx)?)
    }

    fn get_last_batch_idx(&self) -> DbResult<Option<u64>> {
        Ok(rockbound::utils::get_last::<BatchCommitmentSchema>(&*self.db)?.map(|(x, _)| x))
    }
}

pub struct SequencerDB<D> {
    db: Arc<D>,
}

impl<D> SequencerDB<D> {
    pub fn new(db: Arc<D>) -> Self {
        Self { db }
    }
}

impl<D: SeqDataStore + SeqDataProvider> SequencerDatabase for SequencerDB<D> {
    type SeqStore = D;
    type SeqProv = D;

    fn sequencer_store(&self) -> &Arc<Self::SeqStore> {
        &self.db
    }

    fn sequencer_provider(&self) -> &Arc<Self::SeqProv> {
        &self.db
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use alpen_express_db::traits::{SeqDataProvider, SeqDataStore};
    use alpen_express_primitives::buf::Buf32;
    use alpen_test_utils::ArbitraryGenerator;
    use test;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    #[test]
    fn test_put_blob_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [0; 32].into();

        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();
        let idx = seq_db.get_last_blob_idx().unwrap().unwrap();

        assert_eq!(seq_db.get_blob_id(idx).unwrap(), Some(blob_hash));

        let stored_blob = seq_db.get_blob_by_id(blob_hash).unwrap();
        assert_eq!(stored_blob, Some(blob));
    }

    #[test]
    fn test_put_blob_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);
        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [0; 32].into();

        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();

        let result = seq_db.put_blob_entry(blob_hash, blob);

        // Should be ok to put to existing key
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_blob_() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [0; 32].into();

        // Insert
        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();

        let updated_blob: BlobEntry = ArbitraryGenerator::new().generate();

        // Update existing idx
        seq_db
            .put_blob_entry(blob_hash, updated_blob.clone())
            .unwrap();
        let retrieved_blob = seq_db.get_blob_by_id(blob_hash).unwrap().unwrap();
        assert_eq!(updated_blob, retrieved_blob);
    }

    #[test]
    fn test_get_blob_by_id() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [0; 32].into();

        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();

        let retrieved = seq_db.get_blob_by_id(blob_hash).unwrap().unwrap();
        assert_eq!(retrieved, blob);
    }

    #[test]
    fn test_get_last_blob_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [0; 32].into();

        let last_blob_idx = seq_db.get_last_blob_idx().unwrap();
        assert_eq!(
            last_blob_idx, None,
            "There is no last blobidx in the beginning"
        );

        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();
        // Now the last idx is 0

        let blob: BlobEntry = ArbitraryGenerator::new().generate();
        let blob_hash: Buf32 = [1; 32].into();

        seq_db.put_blob_entry(blob_hash, blob.clone()).unwrap();
        // Now the last idx is 1

        let last_blob_idx = seq_db.get_last_blob_idx().unwrap();
        assert_eq!(last_blob_idx, Some(1));
    }

    #[test]
    fn test_batch_commitment_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let batchidx = 1;
        let batch: BatchCommitment = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_commitment(batchidx, batch.clone())
            .unwrap();

        let retrieved_batch = seq_db.get_batch_commitment(batchidx).unwrap().unwrap();
        assert_eq!(batch, retrieved_batch);
    }

    #[test]
    fn test_batch_commitment_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let batchidx = 1;
        let batch: BatchCommitment = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_commitment(batchidx, batch.clone())
            .unwrap();

        let res = seq_db.put_batch_commitment(batchidx, batch.clone());
        assert!(res.is_err_and(|x| matches!(x, DbError::OverwriteBatchCommitment(_batchidx))));
    }

    #[test]
    fn test_batch_commitment_non_monotonic_entries() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let batch: BatchCommitment = ArbitraryGenerator::new().generate();
        seq_db.put_batch_commitment(100, batch.clone()).unwrap();
        seq_db.put_batch_commitment(1, batch.clone()).unwrap();
        seq_db.put_batch_commitment(3, batch.clone()).unwrap();
    }

    #[test]
    fn test_get_last_batch_commitment_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = SeqDb::new(db, db_ops);

        let batch: BatchCommitment = ArbitraryGenerator::new().generate();
        seq_db.put_batch_commitment(100, batch.clone()).unwrap();
        seq_db.put_batch_commitment(1, batch.clone()).unwrap();
        seq_db.put_batch_commitment(3, batch.clone()).unwrap();

        let last_idx = seq_db.get_last_batch_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);

        seq_db.put_batch_commitment(50, batch.clone()).unwrap();
        let last_idx = seq_db.get_last_batch_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);
    }
}

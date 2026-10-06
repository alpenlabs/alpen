use strata_db_types::DbResult;
use strata_db_types::checkpoint_proof::{CheckpointProofDatabase, ProofReceiptEntry};
use strata_db_types::errors::DbError;
use strata_db_types::prover_task::ProverTaskDatabase;
use strata_identifiers::EpochCommitment;
use strata_paas::TaskRecordData;
use typed_sled::tree::SledTreeIter;

use super::schemas::{CheckpointProofSchema, ProverTaskTree};
use crate::define_sled_database;
use crate::utils::conv_sled_err;

define_sled_database!(
    pub struct ProofDBSled {
        checkpoint_proof_tree: CheckpointProofSchema,
        prover_task_tree: ProverTaskTree,
    }
);

impl CheckpointProofDatabase for ProofDBSled {
    fn put_proof(&self, epoch: EpochCommitment, proof: ProofReceiptEntry) -> DbResult<()> {
        // Upsert: a re-prove for the same epoch attests to the same statement,
        // so overwriting is safe and lets the receipt hook be idempotent.
        // Refusing the write would only turn "we already have a valid proof"
        // into a confusing PermanentFailure on the prover task. Matches the EE
        // side's `put_acct_proof`.
        let old = self
            .checkpoint_proof_tree
            .get(&epoch)
            .map_err(conv_sled_err)?;
        self.checkpoint_proof_tree
            .compare_and_swap(epoch, old, Some(proof))
            .map_err(conv_sled_err)?;
        Ok(())
    }

    fn get_proof(&self, epoch: EpochCommitment) -> DbResult<Option<ProofReceiptEntry>> {
        self.checkpoint_proof_tree
            .get(&epoch)
            .map_err(conv_sled_err)
    }

    fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let old = self
            .checkpoint_proof_tree
            .get(&epoch)
            .map_err(conv_sled_err)?;
        let existed = old.is_some();
        self.checkpoint_proof_tree
            .compare_and_swap(epoch, old, None)
            .map_err(conv_sled_err)?;
        Ok(existed)
    }
}

impl ProofDBSled {
    /// Bounds the key range before values are decoded, so another spec's records are not read.
    fn scan_tasks_with_prefix(
        &self,
        prefix: Vec<u8>,
        key_len: usize,
    ) -> DbResult<SledTreeIter<ProverTaskTree>> {
        if key_len < prefix.len() {
            return self
                .prover_task_tree
                .range(prefix.clone()..prefix)
                .map_err(conv_sled_err);
        }
        let mut last_key = prefix.clone();
        last_key.resize(key_len, u8::MAX);
        self.prover_task_tree
            .range(prefix..=last_key)
            .map_err(conv_sled_err)
    }
}

impl ProverTaskDatabase for ProofDBSled {
    fn get_task(&self, key: Vec<u8>) -> DbResult<Option<TaskRecordData>> {
        self.prover_task_tree.get(&key).map_err(conv_sled_err)
    }

    fn insert_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()> {
        if self
            .prover_task_tree
            .get(&key)
            .map_err(conv_sled_err)?
            .is_some()
        {
            return Err(DbError::EntryAlreadyExists);
        }
        self.prover_task_tree
            .compare_and_swap(key, None, Some(record))
            .map_err(conv_sled_err)?;
        Ok(())
    }

    fn put_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()> {
        let old = self.prover_task_tree.get(&key).map_err(conv_sled_err)?;
        self.prover_task_tree
            .compare_and_swap(key, old, Some(record))
            .map_err(conv_sled_err)?;
        Ok(())
    }

    fn delete_task(&self, key: Vec<u8>) -> DbResult<bool> {
        let old = self.prover_task_tree.get(&key).map_err(conv_sled_err)?;
        let existed = old.is_some();
        self.prover_task_tree
            .compare_and_swap(key, old, None)
            .map_err(conv_sled_err)?;
        Ok(existed)
    }

    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            let (key, record) = item.map_err(conv_sled_err)?;
            if record.status().wants_rescan()
                && record.retry_after_secs().is_some_and(|t| t <= now_secs)
            {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn list_unfinished(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            let (key, record) = item.map_err(conv_sled_err)?;
            if record.status().is_unfinished() {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn list_all_tasks(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            out.push(item.map_err(conv_sled_err)?);
        }
        Ok(out)
    }

    fn list_retriable_with_prefix(
        &self,
        prefix: Vec<u8>,
        key_len: usize,
        now_secs: u64,
    ) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.scan_tasks_with_prefix(prefix, key_len)? {
            let (key, record) = item.map_err(conv_sled_err)?;
            if key.len() == key_len
                && record.status().wants_rescan()
                && record.retry_after_secs().is_some_and(|t| t <= now_secs)
            {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn list_unfinished_with_prefix(
        &self,
        prefix: Vec<u8>,
        key_len: usize,
    ) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.scan_tasks_with_prefix(prefix, key_len)? {
            let (key, record) = item.map_err(conv_sled_err)?;
            if key.len() == key_len && record.status().is_unfinished() {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn count_tasks_with_prefix(&self, prefix: Vec<u8>, key_len: usize) -> DbResult<usize> {
        let mut count = 0;
        for item in self.scan_tasks_with_prefix(prefix, key_len)? {
            let (key, _) = item.map_err(conv_sled_err)?;
            if key.len() == key_len {
                count += 1;
            }
        }
        Ok(count)
    }

    fn count_tasks(&self) -> DbResult<usize> {
        let mut n = 0;
        for item in self.prover_task_tree.iter() {
            item.map_err(conv_sled_err)?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::proof_db_tests;
    use strata_paas::{AttemptCounts, TaskStatus};
    use typed_sled::{Schema, SledDb};

    use super::*;
    use crate::{SledDbConfig, sled_db_test_setup};

    sled_db_test_setup!(ProofDBSled, proof_db_tests);

    #[test]
    fn prefix_scans_filter_status_retry_time_and_key_length() {
        let db = setup_db();
        let prefix = vec![0x12, 0xff];
        let key = |suffix| vec![0x12, 0xff, suffix];
        let counts = AttemptCounts::default();
        let transient = || TaskStatus::TransientFailure {
            counts,
            error: "retry".into(),
        };
        let blocked = || TaskStatus::Blocked {
            counts,
            reason: "waiting".into(),
        };
        for (suffix, status, retry_after) in [
            (0, TaskStatus::Pending, None),
            (1, TaskStatus::Proving { counts }, Some(90)),
            (2, transient(), None),
            (3, transient(), Some(99)),
            (4, blocked(), Some(100)),
            (5, blocked(), Some(101)),
            (6, TaskStatus::Completed, Some(90)),
            (
                7,
                TaskStatus::PermanentFailure {
                    error: "done".into(),
                },
                Some(90),
            ),
            (0xff, TaskStatus::Pending, None),
        ] {
            let mut record = TaskRecordData::new(status);
            record.set_retry_after_secs(retry_after);
            db.insert_task(key(suffix), record).unwrap();
        }
        for other_key in [
            vec![0x12, 0xfe, 0],
            vec![0x13, 0, 0],
            prefix.clone(),
            vec![0x12, 0xff, 0, 0],
        ] {
            db.insert_task(other_key, TaskRecordData::new(TaskStatus::Pending))
                .unwrap();
        }
        let keys = |records: Vec<(Vec<u8>, TaskRecordData)>| {
            records.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };
        assert_eq!(
            keys(db.list_unfinished_with_prefix(prefix.clone(), 3).unwrap()),
            vec![key(0), key(1), key(0xff)]
        );
        for (now_secs, expected) in [
            (98, vec![]),
            (99, vec![key(3)]),
            (100, vec![key(3), key(4)]),
            (101, vec![key(3), key(4), key(5)]),
        ] {
            assert_eq!(
                keys(
                    db.list_retriable_with_prefix(prefix.clone(), 3, now_secs)
                        .unwrap()
                ),
                expected
            );
        }
        assert_eq!(db.count_tasks_with_prefix(prefix.clone(), 3).unwrap(), 9);
        assert!(
            db.list_unfinished_with_prefix(prefix.clone(), 1)
                .unwrap()
                .is_empty()
        );
        assert!(
            db.list_retriable_with_prefix(prefix.clone(), 1, 100)
                .unwrap()
                .is_empty()
        );
        assert_eq!(db.count_tasks_with_prefix(prefix, 1).unwrap(), 0);
    }

    #[test]
    fn prefix_scans_do_not_decode_another_namespace() {
        let raw = sled::Config::new().temporary(true).open().unwrap();
        let db = ProofDBSled::new(
            SledDb::new(raw.clone()).unwrap().into(),
            SledDbConfig::test(),
        )
        .unwrap();
        let tree = raw.open_tree(ProverTaskTree::TREE_NAME.0).unwrap();
        for key in [[0, 0, 0, 0, 0], [0, 0, 0, 2, 0]] {
            tree.insert(key, &[0xff]).unwrap();
        }
        let prefix = vec![0, 0, 0, 1];
        let pending_key = vec![0, 0, 0, 1, 0];
        db.insert_task(
            pending_key.clone(),
            TaskRecordData::new(TaskStatus::Pending),
        )
        .unwrap();
        let retry_key = vec![0, 0, 0, 1, 1];
        let mut retry = TaskRecordData::new(TaskStatus::TransientFailure {
            counts: AttemptCounts::default(),
            error: "retry".into(),
        });
        retry.set_retry_after_secs(Some(10));
        db.insert_task(retry_key.clone(), retry).unwrap();
        let unfinished = db.list_unfinished_with_prefix(prefix.clone(), 5).unwrap();
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].0, pending_key);
        let retriable = db
            .list_retriable_with_prefix(prefix.clone(), 5, 10)
            .unwrap();
        assert_eq!(retriable.len(), 1);
        assert_eq!(retriable[0].0, retry_key);
        assert_eq!(db.count_tasks_with_prefix(prefix.clone(), 5).unwrap(), 2);

        // A corrupt value inside the requested namespace must still report an error.
        tree.insert([0, 0, 0, 1, 2], &[0xff]).unwrap();
        assert!(db.list_unfinished_with_prefix(prefix.clone(), 5).is_err());
        assert!(db.list_retriable_with_prefix(prefix, 5, 10).is_err());
    }
}

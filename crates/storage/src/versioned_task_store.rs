//! Spec-scoped persistence for fixed-host checkpoint proving services.

use std::sync::Arc;

use strata_checkpoint_types::CheckpointProofTask;
use strata_ol_state_types::OLSpecId;
use strata_paas::{ProverError, ProverResult, TaskRecord, TaskStatus, TaskStore};

use crate::ProverTaskDbManager;

/// Limits a prover service to tasks for one OL spec.
///
/// Prefixes stored keys with the spec's four-byte identifier and removes it when
/// returning this service's records. Preserves task status, retries, and remote-job metadata.
/// These methods perform blocking database operations.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "the storage manager has no Debug implementation"
)]
pub struct VersionedTaskStore {
    inner: Arc<ProverTaskDbManager>,
    spec: OLSpecId,
}

impl VersionedTaskStore {
    /// Restricts the task database to one fixed-host service's spec.
    pub fn new(inner: Arc<ProverTaskDbManager>, spec: OLSpecId) -> Self {
        Self { inner, spec }
    }

    /// Prepends the spec to a checkpoint task key for storage or offline backfill.
    pub fn encode_key(spec: OLSpecId, task: CheckpointProofTask) -> Vec<u8> {
        let prefix = u32::from(spec).to_be_bytes();
        let task_key = task.to_key_bytes();
        let mut key = Vec::with_capacity(prefix.len() + task_key.len());
        key.extend_from_slice(&prefix);
        key.extend_from_slice(&task_key);
        key
    }

    /// Reads the spec prefix and checkpoint task from a stored key.
    ///
    /// Returns `None` for old unprefixed keys, unknown specs, or malformed tasks.
    pub fn decode_key(key: &[u8]) -> Option<(OLSpecId, CheckpointProofTask)> {
        let (prefix, task_key) = key.split_first_chunk::<4>()?;
        let spec = OLSpecId::try_from(u32::from_be_bytes(*prefix)).ok()?;
        let task = CheckpointProofTask::from_key_bytes(task_key).ok()?;
        Some((spec, task))
    }

    fn prefixed(&self, key: &[u8]) -> ProverResult<Vec<u8>> {
        let task = CheckpointProofTask::from_key_bytes(key)
            .map_err(|error| ProverError::Codec(error.to_string()))?;
        Ok(Self::encode_key(self.spec, task))
    }

    fn strip_prefix(&self, record: TaskRecord) -> Option<TaskRecord> {
        let (spec, task) = Self::decode_key(record.key())?;
        (spec == self.spec)
            .then(|| TaskRecord::from_parts(task.to_key_bytes(), record.data().clone()))
    }
}

impl TaskStore for VersionedTaskStore {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        Ok(self
            .inner
            .get(&self.prefixed(key)?)?
            .and_then(|record| self.strip_prefix(record)))
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let physical_key = self.prefixed(record.key())?;
        self.inner
            .insert(TaskRecord::from_parts(physical_key, record.data().clone()))
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.inner.update_status(&self.prefixed(key)?, status)
    }

    fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
        self.inner.set_retry_after(&self.prefixed(key)?, when_secs)
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.inner.set_metadata(&self.prefixed(key)?, data)
    }

    fn clear_metadata(&self, key: &[u8]) -> ProverResult<()> {
        self.inner.clear_metadata(&self.prefixed(key)?)
    }

    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
        let prefix = u32::from(self.spec).to_be_bytes();
        let key_len = prefix.len() + CheckpointProofTask::KEY_LEN;
        Ok(self
            .inner
            .list_retriable_with_prefix(prefix.to_vec(), key_len, now_secs)?
            .into_iter()
            .filter_map(|record| self.strip_prefix(record))
            .collect())
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        let prefix = u32::from(self.spec).to_be_bytes();
        let key_len = prefix.len() + CheckpointProofTask::KEY_LEN;
        Ok(self
            .inner
            .list_unfinished_with_prefix(prefix.to_vec(), key_len)?
            .into_iter()
            .filter_map(|record| self.strip_prefix(record))
            .collect())
    }

    /// Counts every valid record in this spec's namespace, including terminal tasks.
    ///
    /// Scans this spec's key range without collecting task records. The length check
    /// excludes old unprefixed keys and malformed keys that share the spec prefix.
    fn count(&self) -> ProverResult<usize> {
        let prefix = u32::from(self.spec).to_be_bytes();
        let key_len = prefix.len() + CheckpointProofTask::KEY_LEN;
        self.inner.count_tasks_with_prefix(prefix.to_vec(), key_len)
    }
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, EpochCommitment, OLBlockId};
    use strata_paas::AttemptCounts;

    use super::*;
    use crate::{create_node_storage, test_runtime_handle};

    fn task(epoch: u32) -> CheckpointProofTask {
        CheckpointProofTask(EpochCommitment::new(
            epoch,
            u64::from(epoch),
            OLBlockId::from(Buf32::from([7; 32])),
        ))
    }

    fn stores() -> (
        Arc<ProverTaskDbManager>,
        VersionedTaskStore,
        VersionedTaskStore,
    ) {
        let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
        let raw = Arc::clone(storage.prover_tasks());
        (
            Arc::clone(&raw),
            VersionedTaskStore::new(Arc::clone(&raw), OLSpecId::V1),
            VersionedTaskStore::new(raw, OLSpecId::V0),
        )
    }

    #[test]
    fn stored_keys_prefix_the_unchanged_task_encoding() {
        for spec in [OLSpecId::V0, OLSpecId::V1] {
            let task = task(7);
            let key = VersionedTaskStore::encode_key(spec, task);
            assert_eq!(key.len(), 48);
            assert_eq!(&key[..4], u32::from(spec).to_be_bytes());
            assert_eq!(&key[4..], task.to_key_bytes());
            assert_eq!(VersionedTaskStore::decode_key(&key), Some((spec, task)));
        }
        assert_eq!(
            VersionedTaskStore::decode_key(&task(1).to_key_bytes()),
            None
        );
    }

    #[test]
    fn recovery_and_retry_preserve_metadata_counters_and_timestamps() {
        let (raw, v1, other) = stores();
        let key = task(3).to_key_bytes();
        let counts = AttemptCounts {
            retry: 4,
            resubmit: 2,
            recheck: 6,
        };
        let mut record = TaskRecord::new(key.clone(), TaskStatus::Proving { counts });
        record.data_mut().set_metadata(Some(vec![1, 2, 3]));
        record.data_mut().set_retry_after_secs(Some(100));
        let timestamp = record.data().updated_at_secs();
        v1.insert(record).unwrap();
        let physical_key = VersionedTaskStore::encode_key(OLSpecId::V1, task(3));
        assert!(raw.get(&key).unwrap().is_none());
        assert!(raw.get(&physical_key).unwrap().is_some());
        let recovered = v1.list_unfinished().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].key(), key);
        assert_eq!(recovered[0].metadata(), Some(&[1, 2, 3][..]));
        assert_eq!(recovered[0].status().counts(), counts);
        assert_eq!(recovered[0].retry_after_secs(), Some(100));
        assert_eq!(recovered[0].data().updated_at_secs(), timestamp);
        assert!(other.list_unfinished().unwrap().is_empty());
        v1.update_status(
            &key,
            TaskStatus::TransientFailure {
                counts,
                error: "retry".into(),
            },
        )
        .unwrap();
        assert!(v1.list_retriable(99).unwrap().is_empty());
        let retried = v1.list_retriable(100).unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].key(), key);
        assert_eq!(retried[0].metadata(), Some(&[1, 2, 3][..]));
        assert_eq!(retried[0].status().counts(), counts);
        assert!(other.list_retriable(100).unwrap().is_empty());
        assert_eq!(raw.count().unwrap(), 1);
    }

    #[test]
    fn per_key_mutations_are_isolated_and_scans_return_logical_keys() {
        let (raw, v1, other) = stores();
        let key = task(2).to_key_bytes();
        v1.insert(TaskRecord::new(key.clone(), TaskStatus::Pending))
            .unwrap();
        other
            .insert(TaskRecord::new(key.clone(), TaskStatus::Pending))
            .unwrap();
        other.set_metadata(&key, vec![9]).unwrap();
        other.set_retry_after(&key, 10).unwrap();
        assert_eq!(v1.get(&key).unwrap().unwrap().metadata(), None);
        assert_eq!(v1.get(&key).unwrap().unwrap().retry_after_secs(), None);
        assert_eq!(other.list_unfinished().unwrap()[0].key(), key);
        other
            .update_status(
                &key,
                TaskStatus::Blocked {
                    reason: "waiting".into(),
                    counts: AttemptCounts::default(),
                },
            )
            .unwrap();
        assert_eq!(other.list_retriable(10).unwrap()[0].key(), key);
        assert!(v1.list_retriable(10).unwrap().is_empty());
        assert_eq!(v1.list_unfinished().unwrap().len(), 1);
        other.clear_metadata(&key).unwrap();
        assert_eq!(other.get(&key).unwrap().unwrap().metadata(), None);
        let physical = VersionedTaskStore::encode_key(OLSpecId::V0, task(2));
        assert_eq!(
            raw.get(&physical).unwrap().unwrap().retry_after_secs(),
            Some(10)
        );
        assert_eq!(raw.count().unwrap(), 2);
    }

    #[test]
    fn count_includes_terminal_records_only_in_its_own_namespace() {
        let (_, v1, other) = stores();
        for (epoch, status) in [
            (1, TaskStatus::Pending),
            (2, TaskStatus::Completed),
            (
                3,
                TaskStatus::PermanentFailure {
                    error: "done".into(),
                },
            ),
        ] {
            v1.insert(TaskRecord::new(task(epoch).to_key_bytes(), status))
                .unwrap();
        }
        other
            .insert(TaskRecord::new(
                task(1).to_key_bytes(),
                TaskStatus::Completed,
            ))
            .unwrap();
        assert_eq!(v1.count().unwrap(), 3);
        assert_eq!(other.count().unwrap(), 1);
    }

    #[test]
    fn untagged_unknown_and_malformed_rows_do_not_starve_valid_recovery_or_retry() {
        let (raw, v1, _) = stores();
        let mut unknown = VersionedTaskStore::encode_key(OLSpecId::V0, task(1));
        unknown[3] = 99;
        // Epoch 1's old key starts with the V1 prefix but has no separate spec field.
        let unprefixed = task(1).to_key_bytes();
        assert!(unprefixed.starts_with(&u32::from(OLSpecId::V1).to_be_bytes()));
        let malformed = u32::from(OLSpecId::V1).to_be_bytes().to_vec();
        for key in [unknown, vec![1, 2], unprefixed, malformed] {
            raw.insert(TaskRecord::new(key, TaskStatus::Pending))
                .unwrap();
        }
        v1.insert(TaskRecord::new(task(3).to_key_bytes(), TaskStatus::Pending))
            .unwrap();
        assert_eq!(v1.list_unfinished().unwrap().len(), 1);
        for record in raw.list_all_tasks().unwrap() {
            raw.update_status(
                record.key(),
                TaskStatus::TransientFailure {
                    counts: AttemptCounts::default(),
                    error: "retry".into(),
                },
            )
            .unwrap();
            raw.set_retry_after(record.key(), 10).unwrap();
        }
        let retry = v1.list_retriable(10).unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].key(), task(3).to_key_bytes());
        assert_eq!(v1.count().unwrap(), 1);
        assert_eq!(raw.count().unwrap(), 5);
    }

    #[test]
    fn rejects_physical_or_malformed_keys_in_the_logical_api() {
        let (raw, v1, _) = stores();
        let tagged = VersionedTaskStore::encode_key(OLSpecId::V0, task(1));
        for invalid in [tagged, vec![0]] {
            assert!(matches!(v1.get(&invalid), Err(ProverError::Codec(_))));
            assert!(matches!(
                v1.insert(TaskRecord::new(invalid, TaskStatus::Pending)),
                Err(ProverError::Codec(_))
            ));
        }
        assert_eq!(raw.count().unwrap(), 0);
    }
    mod recovery {
        use std::time::Duration;

        use async_trait::async_trait;
        use strata_paas::{
            InMemoryReceiptStore, InputResolution, ProofSpec, Prover, ProverBuilder,
        };
        use tokio::time::timeout;
        use zkaleido::{
            ProofType, PublicValues, ZkVmEnvSerde, ZkVmHost, ZkVmInputBuilder, ZkVmInputResult,
            ZkVmProgram, ZkVmResult,
        };
        use zkaleido_native_adapter::NativeHost;

        use super::*;

        struct TestProgram;

        impl ZkVmProgram for TestProgram {
            type Input = u32;
            type Output = u32;

            fn name() -> String {
                "checkpoint_namespace_test".into()
            }
            fn proof_type() -> ProofType {
                ProofType::Core
            }
            fn prepare_input<'a, B: ZkVmInputBuilder<'a>>(
                input: &'a u32,
            ) -> ZkVmInputResult<B::Input> {
                B::new().write_serde(input)?.build()
            }
            fn process_output<H: ZkVmHost>(values: &PublicValues) -> ZkVmResult<u32> {
                H::extract_serde_public_output(values)
            }
        }

        struct TestSpec;

        #[async_trait]
        impl ProofSpec for TestSpec {
            type Task = CheckpointProofTask;
            type Program = TestProgram;

            async fn resolve_input(
                &self,
                task: &CheckpointProofTask,
            ) -> ProverResult<InputResolution<u32>> {
                Ok(InputResolution::Ready(task.commitment().epoch))
            }
        }

        fn service(store: VersionedTaskStore, marker: u32) -> Arc<Prover<TestSpec>> {
            let host = NativeHost::new_with_random_key(move |machine| {
                let epoch: u32 = machine.read_serde();
                machine.commit_serde(&(marker + epoch));
            });
            Arc::new(
                ProverBuilder::new(TestSpec)
                    .task_store(store)
                    .receipt_store(InMemoryReceiptStore::new())
                    .native(host),
            )
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn fixed_services_recover_and_retry_only_their_own_persisted_rows() {
            let (raw, v1, other) = stores();
            let tasks = [task(1), task(2)];
            for (store, metadata) in [(&v1, vec![1, 11]), (&other, vec![2, 22])] {
                store
                    .insert(TaskRecord::new(
                        tasks[0].to_key_bytes(),
                        TaskStatus::Pending,
                    ))
                    .unwrap();
                store
                    .insert(TaskRecord::new(
                        tasks[1].to_key_bytes(),
                        TaskStatus::TransientFailure {
                            counts: AttemptCounts {
                                retry: 2,
                                resubmit: 1,
                                recheck: 3,
                            },
                            error: "interrupted".into(),
                        },
                    ))
                    .unwrap();
                store.set_retry_after(&tasks[1].to_key_bytes(), 0).unwrap();
                for task in tasks {
                    store
                        .set_metadata(&task.to_key_bytes(), metadata.clone())
                        .unwrap();
                }
            }
            raw.insert(TaskRecord::new(vec![1, 2], TaskStatus::Pending))
                .unwrap();
            let foreign_before =
                tasks.map(|task| other.get(&task.to_key_bytes()).unwrap().unwrap());
            let first = service(v1.clone(), 100);
            first.tick().await;
            let results = timeout(Duration::from_secs(10), first.wait_for_tasks(&tasks))
                .await
                .unwrap()
                .unwrap();
            assert!(results.iter().all(|result| result.is_completed()));
            for (task, before) in tasks.iter().zip(foreign_before) {
                let own = v1.get(&task.to_key_bytes()).unwrap().unwrap();
                assert_eq!(own.metadata(), Some(&[1, 11][..]));
                let foreign = other.get(&task.to_key_bytes()).unwrap().unwrap();
                assert_eq!(foreign.status(), before.status());
                assert_eq!(foreign.retry_after_secs(), before.retry_after_secs());
                assert_eq!(foreign.metadata(), before.metadata());
                assert_eq!(
                    foreign.data().updated_at_secs(),
                    before.data().updated_at_secs()
                );
            }
            let second = service(other.clone(), 200);
            second.tick().await;
            let results = timeout(Duration::from_secs(10), second.wait_for_tasks(&tasks))
                .await
                .unwrap()
                .unwrap();
            assert!(results.iter().all(|result| result.is_completed()));
            for task in tasks {
                for (prover, marker) in [(&first, 100), (&second, 200)] {
                    let receipt = prover.get_receipt(&task).unwrap().unwrap();
                    assert_eq!(
                        TestProgram::process_output::<NativeHost>(
                            receipt.receipt().public_values()
                        )
                        .unwrap(),
                        marker + task.commitment().epoch
                    );
                }
                assert_eq!(
                    other.get(&task.to_key_bytes()).unwrap().unwrap().metadata(),
                    Some(&[2, 22][..])
                );
            }
            assert_eq!(v1.count().unwrap(), 2);
            assert_eq!(other.count().unwrap(), 2);
            assert_eq!(raw.count().unwrap(), 5);
        }
    }
}

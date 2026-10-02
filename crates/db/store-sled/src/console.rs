//! Console registrations for the concrete Sled backend.

use std::marker::PhantomData;

use sled::transaction::TransactionError;
use strata_db_console::{
    ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleRow, ConsoleScalar, ConsoleTable,
    ConsoleValue, RecordHandle, RecordStream, RegisteredConsoleValue, ScalarType, ScanDirection,
    StagedWrite, ValueMetadata, WriteChange, WritePreview,
};
use strata_identifiers::{Buf32, OLBlockId};
use typed_sled::error::{Error as SledError, Result as SledResult};
use typed_sled::transaction::SledTransactional;
use typed_sled::{Schema, SledTree, ValueCodec};

use crate::SledBackend;

mod views;

type DecodedValue<S> = <<S as Schema>::Value as ValueCodec<S>>::Decoded;

/// Generic console adapter around a typed Sled tree.
pub(crate) struct SledConsoleTable<S, V>
where
    S: Schema,
{
    name: &'static str,
    aliases: &'static [&'static str],
    key_type: ScalarType,
    tree: SledTree<S>,
    parse_key: fn(&ConsoleScalar) -> ConsoleResult<S::Key>,
    render_key: fn(S::Key) -> ConsoleScalar,
    map_value: fn(DecodedValue<S>) -> V,
    unmap_value: Option<fn(V) -> S::Value>,
    _value: PhantomData<V>,
}

impl<S, V> SledConsoleTable<S, V>
where
    S: Schema,
{
    /// Creates a table adapter using the schema's production codecs.
    pub(crate) fn new(
        name: &'static str,
        aliases: &'static [&'static str],
        key_type: ScalarType,
        tree: SledTree<S>,
        parse_key: fn(&ConsoleScalar) -> ConsoleResult<S::Key>,
        render_key: fn(S::Key) -> ConsoleScalar,
        map_value: fn(DecodedValue<S>) -> V,
    ) -> Self {
        Self {
            name,
            aliases,
            key_type,
            tree,
            parse_key,
            render_key,
            map_value,
            unmap_value: None,
            _value: PhantomData,
        }
    }

    /// Enables writes by mapping an edited console value back to the stored schema value.
    pub(crate) fn with_unmap_value(mut self, unmap_value: fn(V) -> S::Value) -> Self {
        self.unmap_value = Some(unmap_value);
        self
    }

    fn make_handle(&self, key: S::Key, value: DecodedValue<S>) -> RecordHandle
    where
        V: ConsoleValue + 'static,
    {
        RecordHandle::new(self.name, (self.render_key)(key), (self.map_value)(value))
    }
}

impl<S, V> SledConsoleTable<S, V>
where
    S: Schema + Clone + Send + Sync + 'static,
    S::Key: Clone + Send + Sync + 'static,
    S::Value: Clone + Send + Sync + 'static,
    DecodedValue<S>: 'static,
    V: RegisteredConsoleValue + Clone + Send + Sync + 'static,
{
    /// Prepares one typed replacement and its stable before/after preview.
    pub(crate) fn stage_write(
        &self,
        key: &ConsoleScalar,
        operation: String,
        mutate: impl FnOnce(&mut V) -> ConsoleResult<()>,
    ) -> ConsoleResult<Box<dyn StagedWrite>> {
        let (entry, change) = self.prepare_write(key, mutate)?;

        Ok(Box::new(SledStagedWrite::<S> {
            name: self.name,
            tree: self.tree.clone(),
            entry,
            preview: WritePreview {
                operation,
                changes: vec![change],
            },
        }))
    }

    /// Prepares an atomic same-table batch and its stable preview.
    pub(crate) fn stage_bulk_write(
        &self,
        keys: &[ConsoleScalar],
        operation: String,
        mutate: impl Fn(&mut V) -> ConsoleResult<()>,
    ) -> ConsoleResult<Box<dyn StagedWrite>> {
        if keys.is_empty() {
            return Err(ConsoleError::invalid_input(
                "bulk modification",
                "at least one key is required",
            ));
        }

        let mut entries = Vec::with_capacity(keys.len());
        let mut changes = Vec::with_capacity(keys.len());
        for key in keys {
            let (entry, change) = self.prepare_write(key, &mutate)?;
            entries.push(entry);
            changes.push(change);
        }

        Ok(Box::new(SledBulkStagedWrite::<S, V> {
            name: self.name,
            tree: self.tree.clone(),
            entries,
            map_value: self.map_value,
            unmap_value: self
                .unmap_value
                .expect("prepared writes require an unmap function"),
            preview: WritePreview { operation, changes },
        }))
    }

    fn prepare_write(
        &self,
        key: &ConsoleScalar,
        mutate: impl FnOnce(&mut V) -> ConsoleResult<()>,
    ) -> ConsoleResult<(SledWriteEntry<S>, WriteChange)> {
        let unmap_value = self
            .unmap_value
            .ok_or(ConsoleError::ReadOnlyTable(self.name))?;
        let key = (self.parse_key)(key)?;
        let decoded = self
            .tree
            .get(&key)
            .map_err(|error| ConsoleError::storage(self.name, error))?
            .ok_or(ConsoleError::MissingRecord { table: self.name })?;
        let original = (self.map_value)(decoded);
        let mut replacement = original.clone();
        mutate(&mut replacement)?;

        let rendered_key = (self.render_key)(key.clone());
        let before = ConsoleRow::from_handle(RecordHandle::new(
            self.name,
            rendered_key.clone(),
            original.clone(),
        ))?;
        let after = ConsoleRow::from_handle(RecordHandle::new(
            self.name,
            rendered_key,
            replacement.clone(),
        ))?;
        Ok((
            SledWriteEntry {
                key,
                expected: unmap_value(original),
                replacement: unmap_value(replacement),
            },
            WriteChange { before, after },
        ))
    }
}

struct SledWriteEntry<S>
where
    S: Schema,
{
    key: S::Key,
    expected: S::Value,
    replacement: S::Value,
}

struct SledStagedWrite<S>
where
    S: Schema,
{
    name: &'static str,
    tree: SledTree<S>,
    entry: SledWriteEntry<S>,
    preview: WritePreview,
}

impl<S> StagedWrite for SledStagedWrite<S>
where
    S: Schema + Send + Sync + 'static,
    S::Key: Clone + Send + Sync + 'static,
    S::Value: Clone + Send + Sync + 'static,
{
    fn preview(&self) -> &WritePreview {
        &self.preview
    }

    fn commit(&self) -> ConsoleResult<()> {
        self.tree
            .compare_and_swap(
                self.entry.key.clone(),
                Some(self.entry.expected.clone()),
                Some(self.entry.replacement.clone()),
            )
            .map_err(|error| match error {
                SledError::CASError(_) => ConsoleError::StaleWrite { table: self.name },
                error => ConsoleError::storage(self.name, error),
            })
    }
}

struct SledBulkStagedWrite<S, V>
where
    S: Schema,
{
    name: &'static str,
    tree: SledTree<S>,
    entries: Vec<SledWriteEntry<S>>,
    map_value: fn(DecodedValue<S>) -> V,
    unmap_value: fn(V) -> S::Value,
    preview: WritePreview,
}

#[derive(Debug, thiserror::Error)]
#[error("a staged record changed before the batch commit")]
struct StaleBatch;

impl<S, V> StagedWrite for SledBulkStagedWrite<S, V>
where
    S: Schema + Send + Sync + 'static,
    S::Key: Clone + Send + Sync + 'static,
    S::Value: Clone + Send + Sync + 'static,
    DecodedValue<S>: 'static,
    V: Send + Sync + 'static,
{
    fn preview(&self) -> &WritePreview {
        &self.preview
    }

    fn commit(&self) -> ConsoleResult<()> {
        let result: Result<(), TransactionError<SledError>> =
            (&self.tree,).transaction(|(tree,)| {
                for entry in &self.entries {
                    let Some(current) = tree.get(&entry.key)? else {
                        return Err(SledError::abort(StaleBatch).into());
                    };
                    let current = (self.unmap_value)((self.map_value)(current));
                    if current.encode_value().map_err(SledError::from)?
                        != entry.expected.encode_value().map_err(SledError::from)?
                    {
                        return Err(SledError::abort(StaleBatch).into());
                    }
                }
                for entry in &self.entries {
                    tree.insert(&entry.key, &entry.replacement)?;
                }
                Ok(())
            });

        result.map_err(|error| match error {
            TransactionError::Abort(error)
                if error.downcast_abort_ref::<StaleBatch>().is_some() =>
            {
                ConsoleError::StaleWrite { table: self.name }
            }
            TransactionError::Abort(error) => ConsoleError::storage(self.name, error),
            TransactionError::Storage(error) => ConsoleError::storage(self.name, error),
        })
    }
}

impl<S, V> ConsoleTable for SledConsoleTable<S, V>
where
    S: Schema + Send + Sync + 'static,
    S::Key: 'static,
    DecodedValue<S>: 'static,
    V: RegisteredConsoleValue + 'static,
{
    fn name(&self) -> &'static str {
        self.name
    }

    fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }

    fn key_type(&self) -> ScalarType {
        self.key_type
    }

    fn value_metadata(&self) -> &'static ValueMetadata {
        V::value_metadata()
    }

    fn get(&self, key: &ConsoleScalar) -> ConsoleResult<Option<RecordHandle>> {
        let key = (self.parse_key)(key)?;
        let value = self
            .tree
            .get(&key)
            .map_err(|error| ConsoleError::storage(self.name, error))?;
        Ok(value.map(|value| self.make_handle(key, value)))
    }

    fn scan(&self, direction: ScanDirection) -> ConsoleResult<RecordStream> {
        let table_name = self.name;
        let render_key = self.render_key;
        let map_value = self.map_value;
        let map_item = move |item: SledResult<(S::Key, DecodedValue<S>)>| {
            item.map(|(key, value)| {
                RecordHandle::new(table_name, render_key(key), map_value(value))
            })
            .map_err(|error| ConsoleError::storage(table_name, error))
        };

        let iterator = self.tree.iter();
        match direction {
            ScanDirection::Forward => Ok(Box::new(iterator.map(map_item))),
            // `SledTreeIter::next_back` delegates to `sled::Iter::next_back`, so this remains a
            // lazy native reverse traversal rather than buffering and reversing the full table.
            ScanDirection::Reverse => Ok(Box::new(iterator.rev().map(map_item))),
        }
    }
}

pub(crate) fn identity<T>(value: T) -> T {
    value
}

pub(crate) fn parse_byte_key(key: &ConsoleScalar) -> ConsoleResult<Vec<u8>> {
    key.as_bytes("table key").map(<[u8]>::to_vec)
}

pub(crate) fn render_byte_key(key: Vec<u8>) -> ConsoleScalar {
    ConsoleScalar::Bytes(key)
}

pub(crate) fn parse_ol_block_id(key: &ConsoleScalar) -> ConsoleResult<OLBlockId> {
    let bytes = key.as_bytes("OL block id")?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        ConsoleError::invalid_input(
            "OL block id",
            format!("expected 32 bytes, got {}", bytes.len()),
        )
    })?;
    Ok(OLBlockId::from(Buf32::from(bytes)))
}

/// Builds the explicit set of Sled tables supported by the console spike.
pub fn build_console_registry(backend: &SledBackend) -> ConsoleResult<ConsoleRegistry> {
    let mut registry = ConsoleRegistry::new();
    for table in backend.prover_db.console_tables() {
        registry.register(table)?;
    }
    for view in views::console_views(backend) {
        registry.register_view(view)?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use strata_db_console::{
        BinaryOperator, ConsoleError, ConsoleExecutor, ConsoleOutput, ConsolePlan, ConsoleScalar,
        PipelinePlan, PipelineTerminal, RecordFormat, ScalarExpression, ScanDirection, Selection,
        SourceKind, WriteSession, render_record, write_json_lines,
    };
    use strata_db_types::ol_block::{BlockStatus, OLBlockDatabase};
    use strata_db_types::prover_task::ProverTaskDatabase;
    use strata_ol_chain_types_v1::test_utils::ol_block_strategy;
    use strata_paas::{TaskRecordData, TaskStatus};

    use super::build_console_registry;
    use crate::test_utils::get_test_sled_db;
    use crate::{SledBackend, SledDbConfig};

    #[test]
    fn registry_maps_a_writable_table_and_native_view() {
        let backend = SledBackend::new(Arc::new(get_test_sled_db()), SledDbConfig::test())
            .expect("test: create Sled backend");

        let mut runner = TestRunner::deterministic();
        let block = ol_block_strategy()
            .new_tree(&mut runner)
            .expect("test: create OL block value tree")
            .current();
        let block_id = block.header().compute_blkid();
        let block_slot = block.header().slot();
        backend
            .ol_block_db
            .put_block_data(block)
            .expect("test: insert OL block");
        backend
            .ol_block_db
            .set_block_status(block_id, BlockStatus::Valid)
            .expect("test: update OL block status");
        for key in [vec![1], vec![2], vec![3]] {
            backend
                .prover_db
                .put_task(key, TaskRecordData::new(TaskStatus::Pending))
                .expect("test: insert prover task");
        }

        let registry = build_console_registry(&backend).expect("test: build console registry");
        let block_key = ConsoleScalar::Bytes(block_id.as_ref().to_vec());

        let task_table = registry.table("ProverTask").expect("test: task table");
        let task = task_table
            .get(&ConsoleScalar::Bytes(vec![1]))
            .expect("test: read prover task")
            .expect("test: prover task exists");
        assert_eq!(
            task.get("status").expect("test: task status getter"),
            ConsoleScalar::String("pending".to_owned())
        );
        assert_eq!(task.metadata().name, "TaskRecordData");
        assert_eq!(task.metadata().fields.len(), 4);
        assert!(
            task.metadata()
                .fields
                .iter()
                .find(|field| field.name == "retry_after_secs")
                .expect("test: retry field metadata")
                .settable
        );
        assert_eq!(task_table.modifiers().len(), 2);

        let forward = task_table
            .scan(ScanDirection::Forward)
            .expect("test: scan tasks")
            .take(2)
            .collect::<Result<Vec<_>, _>>()
            .expect("test: decode first two tasks");
        assert_eq!(forward.len(), 2);
        assert_eq!(forward[0].key(), &ConsoleScalar::Bytes(vec![1]));

        let last = task_table
            .scan(ScanDirection::Reverse)
            .expect("test: reverse scan tasks")
            .next()
            .expect("test: reverse scan item")
            .expect("test: decode reverse scan item");
        assert_eq!(last.key(), &ConsoleScalar::Bytes(vec![3]));

        let executor = ConsoleExecutor::new(&registry);
        let ConsoleOutput::Schema(task_schema) = executor
            .execute(ConsolePlan::schema("tasks"))
            .expect("test: task schema")
        else {
            panic!("test: schema output expected");
        };
        assert_eq!(task_schema.name, "ProverTask");
        assert_eq!(task_schema.kind, SourceKind::Table);

        let ConsoleOutput::Row(block) = executor
            .execute(ConsolePlan::get("OlBlock", vec![block_key]))
            .expect("test: OL block view")
        else {
            panic!("test: record output expected");
        };
        let block = block.expect("test: OL block view exists");
        assert_eq!(
            block.fields.get("status"),
            Some(&ConsoleScalar::String("valid".to_owned()))
        );
        assert_eq!(
            block.fields.get("slot"),
            Some(&ConsoleScalar::U64(block_slot))
        );
        let block_json = render_record(&block, RecordFormat::Json).expect("test: render block");
        assert!(block_json.contains("\"status\": \"valid\""));

        let ConsoleOutput::Rows(task_rows) = executor
            .execute(ConsolePlan::scan("tasks", 2).expect("test: bounded task scan"))
            .expect("test: scan tasks through executor")
        else {
            panic!("test: rows output expected");
        };
        let mut json_lines = Vec::new();
        write_json_lines(task_rows, &mut json_lines).expect("test: render task JSON Lines");
        assert_eq!(
            String::from_utf8(json_lines)
                .expect("test: UTF-8 JSON Lines")
                .lines()
                .count(),
            2
        );

        let ConsoleOutput::Schema(block_schema) = executor
            .execute(ConsolePlan::schema("OlBlock"))
            .expect("test: OL block schema")
        else {
            panic!("test: schema output expected");
        };
        assert_eq!(block_schema.kind, SourceKind::View);
        assert_eq!(block_schema.arguments.len(), 1);
    }

    #[test]
    fn point_writes_preview_abort_commit_and_reject_stale_values() {
        let backend = SledBackend::new(Arc::new(get_test_sled_db()), SledDbConfig::test())
            .expect("test: create Sled backend");
        let key = vec![7];
        backend
            .prover_db
            .put_task(key.clone(), TaskRecordData::new(TaskStatus::Pending))
            .expect("test: insert prover task");

        let registry = build_console_registry(&backend).expect("test: build console registry");
        let console_key = ConsoleScalar::Bytes(key.clone());
        let mut session = WriteSession::new(&registry);

        let preview = session
            .stage_set(
                "tasks",
                &console_key,
                "retry_after_secs",
                &ConsoleScalar::U64(42),
            )
            .expect("test: stage retry setter");
        assert_eq!(preview.operation, "set retry_after_secs");
        assert_eq!(
            preview.changes[0].after.fields.get("retry_after_secs"),
            Some(&ConsoleScalar::U64(42))
        );
        assert_eq!(
            backend
                .prover_db
                .get_task(key.clone())
                .expect("test: read uncommitted task")
                .expect("test: task exists")
                .retry_after_secs(),
            None
        );
        session.abort().expect("test: abort staged setter");

        session
            .stage_set(
                "ProverTask",
                &console_key,
                "retry_after_secs",
                &ConsoleScalar::U64(42),
            )
            .expect("test: restage retry setter");
        let mut concurrent = backend
            .prover_db
            .get_task(key.clone())
            .expect("test: read concurrent task")
            .expect("test: concurrent task exists");
        concurrent.set_retry_after_secs(Some(7));
        backend
            .prover_db
            .put_task(key.clone(), concurrent)
            .expect("test: write concurrent task change");
        assert!(matches!(
            session.commit(),
            Err(ConsoleError::StaleWrite {
                table: "ProverTask"
            })
        ));
        assert!(session.staged().is_some());
        session.abort().expect("test: abort stale setter");

        session
            .stage_set(
                "ProverTask",
                &console_key,
                "retry_after_secs",
                &ConsoleScalar::U64(42),
            )
            .expect("test: stage current retry setter");
        session.commit().expect("test: commit retry setter");
        assert_eq!(
            backend
                .prover_db
                .get_task(key.clone())
                .expect("test: read committed task")
                .expect("test: committed task exists")
                .retry_after_secs(),
            Some(42)
        );

        let preview = session
            .stage_modify(
                "ProverTask",
                &console_key,
                "abandon",
                &[ConsoleScalar::String("operator cancelled".to_owned())],
            )
            .expect("test: stage abandon modifier");
        assert_eq!(
            preview.changes[0].after.fields.get("status"),
            Some(&ConsoleScalar::String("permanent_failure".to_owned()))
        );
        session.commit().expect("test: commit abandon modifier");
        assert!(matches!(
            backend
                .prover_db
                .get_task(key)
                .expect("test: read abandoned task")
                .expect("test: abandoned task exists")
                .status(),
            TaskStatus::PermanentFailure { error } if error == "operator cancelled"
        ));
    }

    #[test]
    fn functional_pipelines_filter_project_and_use_fixed_aggregates() {
        let backend = SledBackend::new(Arc::new(get_test_sled_db()), SledDbConfig::test())
            .expect("test: create Sled backend");
        for (key, status, retry_after_secs) in [
            (vec![1], TaskStatus::Pending, 3),
            (vec![2], TaskStatus::Completed, 11),
            (vec![3], TaskStatus::Pending, 7),
        ] {
            let mut task = TaskRecordData::new(status);
            task.set_retry_after_secs(Some(retry_after_secs));
            backend
                .prover_db
                .put_task(key, task)
                .expect("test: insert prover task");
        }

        let registry = build_console_registry(&backend).expect("test: build console registry");
        let executor = ConsoleExecutor::new(&registry);
        let pending = ScalarExpression::binary(
            BinaryOperator::Equal,
            ScalarExpression::field("status"),
            ScalarExpression::literal(ConsoleScalar::String("pending".to_owned())),
        );
        let base = PipelinePlan::scan("tasks", 10)
            .expect("test: build pipeline")
            .filter(pending);

        let ConsoleOutput::Rows(mut rows) = executor
            .execute(
                base.clone()
                    .select(vec![
                        Selection::new("key", ScalarExpression::Key),
                        Selection::new(
                            "retry_plus_four",
                            ScalarExpression::binary(
                                BinaryOperator::Add,
                                ScalarExpression::field("retry_after_secs"),
                                ScalarExpression::literal(ConsoleScalar::U64(4)),
                            ),
                        ),
                    ])
                    .take(1)
                    .expect("test: add take limit"),
            )
            .expect("test: execute projected pipeline")
        else {
            panic!("test: row output expected");
        };
        let first = rows
            .next()
            .expect("test: projected row")
            .expect("test: evaluate projected row");
        assert_eq!(first.key, ConsoleScalar::Bytes(vec![1]));
        assert_eq!(
            first.fields.get("retry_plus_four"),
            Some(&ConsoleScalar::U64(7))
        );
        assert!(rows.next().is_none());

        let aggregates = [
            (PipelineTerminal::Count, ConsoleScalar::U64(2)),
            (
                PipelineTerminal::Sum(ScalarExpression::field("retry_after_secs")),
                ConsoleScalar::U64(10),
            ),
            (
                PipelineTerminal::Min(ScalarExpression::field("retry_after_secs")),
                ConsoleScalar::U64(3),
            ),
            (
                PipelineTerminal::Max(ScalarExpression::field("retry_after_secs")),
                ConsoleScalar::U64(7),
            ),
            (
                PipelineTerminal::Any(ScalarExpression::binary(
                    BinaryOperator::Greater,
                    ScalarExpression::field("retry_after_secs"),
                    ScalarExpression::literal(ConsoleScalar::U64(5)),
                )),
                ConsoleScalar::Bool(true),
            ),
            (
                PipelineTerminal::All(ScalarExpression::binary(
                    BinaryOperator::Greater,
                    ScalarExpression::field("retry_after_secs"),
                    ScalarExpression::literal(ConsoleScalar::U64(0)),
                )),
                ConsoleScalar::Bool(true),
            ),
        ];
        for (terminal, expected) in aggregates {
            let ConsoleOutput::Scalar(actual) = executor
                .execute(base.clone().terminal(terminal))
                .expect("test: execute aggregate")
            else {
                panic!("test: scalar output expected");
            };
            assert_eq!(actual, expected);
        }

        let ConsoleOutput::Row(last) = executor
            .execute(base.terminal(PipelineTerminal::Last))
            .expect("test: execute last")
        else {
            panic!("test: row output expected");
        };
        assert_eq!(
            last.expect("test: last matching row").key,
            ConsoleScalar::Bytes(vec![3])
        );
    }

    #[test]
    fn bounded_bulk_modification_stages_all_or_nothing_and_commits_atomically() {
        let backend = SledBackend::new(Arc::new(get_test_sled_db()), SledDbConfig::test())
            .expect("test: create Sled backend");
        backend
            .prover_db
            .put_task(vec![1], TaskRecordData::new(TaskStatus::Pending))
            .expect("test: insert pending task");
        backend
            .prover_db
            .put_task(vec![2], TaskRecordData::new(TaskStatus::Completed))
            .expect("test: insert completed task");

        let registry = build_console_registry(&backend).expect("test: build console registry");
        let mut session = WriteSession::new(&registry);
        let arguments = [ConsoleScalar::String("operator cancelled".to_owned())];
        let all_tasks = PipelinePlan::scan("tasks", 2).expect("test: build bounded scan");

        assert!(
            session
                .stage_modify_scan(all_tasks.clone(), "abandon", &arguments)
                .is_err()
        );
        assert!(session.staged().is_none());
        assert!(matches!(
            backend
                .prover_db
                .get_task(vec![1])
                .expect("test: read pending task")
                .expect("test: pending task exists")
                .status(),
            TaskStatus::Pending
        ));

        backend
            .prover_db
            .put_task(vec![2], TaskRecordData::new(TaskStatus::Pending))
            .expect("test: make second task modifiable");
        let preview = session
            .stage_modify_scan(all_tasks, "abandon", &arguments)
            .expect("test: stage two task modifications");
        assert_eq!(preview.changes.len(), 2);

        let mut concurrent = backend
            .prover_db
            .get_task(vec![2])
            .expect("test: read concurrent task")
            .expect("test: concurrent task exists");
        concurrent.set_retry_after_secs(Some(9));
        backend
            .prover_db
            .put_task(vec![2], concurrent)
            .expect("test: write concurrent task change");
        assert!(matches!(
            session.commit(),
            Err(ConsoleError::StaleWrite {
                table: "ProverTask"
            })
        ));
        assert!(matches!(
            backend
                .prover_db
                .get_task(vec![1])
                .expect("test: read first task after stale commit")
                .expect("test: first task exists")
                .status(),
            TaskStatus::Pending
        ));
        session.abort().expect("test: abort stale bulk write");

        let first_task = PipelinePlan::scan("tasks", 2)
            .expect("test: build filtered scan")
            .filter(ScalarExpression::binary(
                BinaryOperator::Equal,
                ScalarExpression::Key,
                ScalarExpression::literal(ConsoleScalar::Bytes(vec![1])),
            ));
        session
            .stage_modify_scan(first_task, "abandon", &arguments)
            .expect("test: stage filtered modification");
        session
            .commit()
            .expect("test: commit filtered modification");
        assert!(matches!(
            backend
                .prover_db
                .get_task(vec![1])
                .expect("test: read abandoned task")
                .expect("test: abandoned task exists")
                .status(),
            TaskStatus::PermanentFailure { error } if error == "operator cancelled"
        ));
    }
}

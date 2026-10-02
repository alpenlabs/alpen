//! Console registrations for the concrete Sled backend.

use std::marker::PhantomData;

use strata_db_console::{
    ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleRow, ConsoleScalar, ConsoleTable,
    ConsoleValue, RecordHandle, RecordStream, RegisteredConsoleValue, ScalarType, ScanDirection,
    StagedWrite, ValueMetadata, WritePreview,
};
use strata_identifiers::{Buf32, OLBlockId};
use typed_sled::error::{Error as SledError, Result as SledResult};
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

        Ok(Box::new(SledStagedWrite::<S> {
            name: self.name,
            tree: self.tree.clone(),
            key,
            expected: unmap_value(original),
            replacement: unmap_value(replacement),
            preview: WritePreview {
                operation,
                before,
                after,
            },
        }))
    }
}

struct SledStagedWrite<S>
where
    S: Schema,
{
    name: &'static str,
    tree: SledTree<S>,
    key: S::Key,
    expected: S::Value,
    replacement: S::Value,
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
                self.key.clone(),
                Some(self.expected.clone()),
                Some(self.replacement.clone()),
            )
            .map_err(|error| match error {
                SledError::CASError(_) => ConsoleError::StaleWrite { table: self.name },
                error => ConsoleError::storage(self.name, error),
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

pub(crate) fn render_ol_block_id(key: OLBlockId) -> ConsoleScalar {
    ConsoleScalar::Bytes(key.as_ref().to_vec())
}

/// Builds the explicit set of Sled tables supported by the console spike.
pub fn build_console_registry(backend: &SledBackend) -> ConsoleResult<ConsoleRegistry> {
    let mut registry = ConsoleRegistry::new();
    for table in backend.ol_block_db.console_tables() {
        registry.register(table)?;
    }
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
        ConsoleError, ConsoleScalar, ReadExecutor, ReadOutput, ReadPlan, RecordFormat,
        ScanDirection, SourceKind, WriteSession, render_record, write_json_lines,
    };
    use strata_db_types::l1::L1Database;
    use strata_db_types::ol_block::{BlockStatus, OLBlockDatabase};
    use strata_db_types::ol_state::OLStateDatabase;
    use strata_db_types::prover_task::ProverTaskDatabase;
    use strata_identifiers::OLBlockCommitment;
    use strata_ol_chain_types_v1::test_utils::ol_block_strategy;
    use strata_ol_state_container::test_utils::create_test_container_with_staged;
    use strata_paas::{TaskRecordData, TaskStatus};
    use strata_primitives::l1::L1BlockId;

    use super::build_console_registry;
    use crate::test_utils::get_test_sled_db;
    use crate::{SledBackend, SledDbConfig};

    #[test]
    fn registry_maps_simple_complex_and_adapter_values() {
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
        backend
            .ol_block_db
            .replace_canonical_suffix_from(block_slot, vec![block_id])
            .expect("test: set canonical OL tip");
        backend
            .ol_state_db
            .put_toplevel_ol_state(
                OLBlockCommitment::new(block_slot, block_id),
                create_test_container_with_staged(1),
            )
            .expect("test: insert canonical OL state");
        backend
            .l1_db
            .set_canonical_chain_entry(100, L1BlockId::default())
            .expect("test: set canonical L1 tip");

        for key in [vec![1], vec![2], vec![3]] {
            backend
                .prover_db
                .put_task(key, TaskRecordData::new(TaskStatus::Pending))
                .expect("test: insert prover task");
        }

        let registry = build_console_registry(&backend).expect("test: build console registry");
        let block_key = ConsoleScalar::Bytes(block_id.as_ref().to_vec());

        let status = registry
            .table("OLBlockStatus")
            .expect("test: status table")
            .get(&block_key)
            .expect("test: read block status")
            .expect("test: block status exists");
        assert_eq!(
            status.get("status").expect("test: status getter"),
            ConsoleScalar::String("valid".to_owned())
        );

        let block = registry
            .table("OLBlock")
            .expect("test: block table")
            .get(&block_key)
            .expect("test: read OL block")
            .expect("test: OL block exists");
        assert_eq!(
            block.get("slot").expect("test: slot getter"),
            ConsoleScalar::U64(block_slot)
        );

        let task_table = registry.table("ProverTask").expect("test: task table");
        let mut task = task_table
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
        assert!(matches!(
            task.set("status", &ConsoleScalar::String("completed".to_owned())),
            Err(ConsoleError::ReadOnlyField { .. })
        ));
        assert_eq!(task_table.modifiers().len(), 2);
        task.set("retry_after_secs", &ConsoleScalar::U64(42))
            .expect("test: set task retry time through domain setter");
        assert_eq!(
            task.get("retry_after_secs")
                .expect("test: updated retry time getter"),
            ConsoleScalar::U64(42)
        );
        task.set("retry_after_secs", &ConsoleScalar::Null)
            .expect("test: clear task retry time through domain setter");
        assert_eq!(
            task.get("retry_after_secs")
                .expect("test: cleared retry time getter"),
            ConsoleScalar::Null
        );
        task_table
            .modify(
                &mut task,
                "abandon",
                &[ConsoleScalar::String("operator cancelled".to_owned())],
            )
            .expect("test: abandon task handle");
        assert_eq!(
            task.get("status").expect("test: modified status getter"),
            ConsoleScalar::String("permanent_failure".to_owned())
        );

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

        let executor = ReadExecutor::new(&registry);
        let ReadOutput::Schema(task_schema) = executor
            .execute(ReadPlan::schema("tasks"))
            .expect("test: task schema")
        else {
            panic!("test: schema output expected");
        };
        assert_eq!(task_schema.name, "ProverTask");
        assert_eq!(task_schema.kind, SourceKind::Table);

        let ReadOutput::Record(block) = executor
            .execute(ReadPlan::get("OlBlock", vec![block_key]))
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

        let ReadOutput::Rows(task_rows) = executor
            .execute(ReadPlan::scan("tasks", 2).expect("test: bounded task scan"))
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

        let ReadOutput::Schema(sync_schema) = executor
            .execute(ReadPlan::schema("SyncInfo"))
            .expect("test: sync-info schema")
        else {
            panic!("test: schema output expected");
        };
        assert_eq!(sync_schema.kind, SourceKind::View);
        assert_eq!(sync_schema.arguments.len(), 1);

        let ReadOutput::Record(sync_info) = executor
            .execute(ReadPlan::get("SyncInfo", vec![ConsoleScalar::U64(6)]))
            .expect("test: sync-info view")
        else {
            panic!("test: record output expected");
        };
        let sync_info = sync_info.expect("test: sync-info value");
        assert_eq!(
            sync_info.fields.get("l1_tip_height"),
            Some(&ConsoleScalar::U64(100))
        );
        assert_eq!(
            sync_info.fields.get("ol_tip_block_id"),
            Some(&ConsoleScalar::Bytes(block_id.as_ref().to_vec()))
        );
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
            preview.after.fields.get("retry_after_secs"),
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
            preview.after.fields.get("status"),
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
}

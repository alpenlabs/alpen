//! Typed read plans and their storage-independent executor.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;

use serde::Serialize;

use crate::{
    ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleScalar, RecordHandle, ScanDirection,
    SourceSchema,
};

/// Stable scalar row produced from a registered value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConsoleRow {
    /// Primary table or view name.
    pub source: &'static str,
    /// Table key or [`ConsoleScalar::Null`] for a keyless view.
    pub key: ConsoleScalar,
    /// Approved fields keyed by their stable console names.
    pub fields: BTreeMap<&'static str, ConsoleScalar>,
}

impl ConsoleRow {
    /// Materializes approved scalar fields from an owned value handle.
    pub fn from_handle(handle: RecordHandle) -> ConsoleResult<Self> {
        let mut fields = BTreeMap::new();
        for descriptor in handle.metadata().fields {
            fields.insert(descriptor.name, handle.get(descriptor.name)?);
        }
        Ok(Self {
            source: handle.table(),
            key: handle.key().clone(),
            fields,
        })
    }
}

/// A bounded table scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanPlan {
    table: String,
    direction: ScanDirection,
    limit: NonZeroUsize,
}

impl ScanPlan {
    /// Creates a bounded scan plan.
    pub fn new(
        table: impl Into<String>,
        direction: ScanDirection,
        limit: usize,
    ) -> ConsoleResult<Self> {
        let limit = NonZeroUsize::new(limit).ok_or_else(|| {
            ConsoleError::invalid_input("scan limit", "must be greater than zero")
        })?;
        Ok(Self {
            table: table.into(),
            direction,
            limit,
        })
    }

    /// Returns the requested table name or alias.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Returns the storage traversal direction.
    pub const fn direction(&self) -> ScanDirection {
        self.direction
    }

    /// Returns the maximum number of records pulled from storage.
    pub const fn limit(&self) -> NonZeroUsize {
        self.limit
    }
}

/// A storage-independent database read assembled directly in Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadPlan {
    /// Describes one registered table or native view.
    Schema {
        /// Primary name or alias.
        source: String,
    },
    /// Reads one table record or executes one native view.
    Get {
        /// Primary name or alias.
        source: String,
        /// A table key or the native view's ordered arguments.
        arguments: Vec<ConsoleScalar>,
    },
    /// Lazily reads a bounded table range.
    Scan(ScanPlan),
}

impl ReadPlan {
    /// Creates a schema plan.
    pub fn schema(source: impl Into<String>) -> Self {
        Self::Schema {
            source: source.into(),
        }
    }

    /// Creates a point-read or native-view plan.
    pub fn get(source: impl Into<String>, arguments: Vec<ConsoleScalar>) -> Self {
        Self::Get {
            source: source.into(),
            arguments,
        }
    }

    /// Creates a bounded forward scan plan.
    pub fn scan(table: impl Into<String>, limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Forward, limit)
    }

    /// Creates a bounded reverse scan plan.
    pub fn scan_rev(table: impl Into<String>, limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Reverse, limit)
    }

    /// Creates a bounded scan plan with an explicit direction.
    pub fn scan_direction(
        table: impl Into<String>,
        direction: ScanDirection,
        limit: usize,
    ) -> ConsoleResult<Self> {
        ScanPlan::new(table, direction, limit).map(Self::Scan)
    }
}

/// Lazy rows returned by a bounded scan.
pub type RowStream = Box<dyn Iterator<Item = ConsoleResult<ConsoleRow>>>;

/// Output of one typed read plan.
pub enum ReadOutput {
    /// Metadata for one registered source.
    Schema(SourceSchema),
    /// One point-read or native-view result.
    Record(Option<ConsoleRow>),
    /// A lazy, bounded stream of table rows.
    Rows(RowStream),
}

impl fmt::Debug for ReadOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(schema) => formatter.debug_tuple("Schema").field(schema).finish(),
            Self::Record(record) => formatter.debug_tuple("Record").field(record).finish(),
            Self::Rows(_) => formatter.write_str("Rows(..)"),
        }
    }
}

/// Executes typed read plans against an explicit registry.
#[derive(Debug)]
pub struct ReadExecutor<'a> {
    registry: &'a ConsoleRegistry,
}

impl<'a> ReadExecutor<'a> {
    /// Creates an executor over a completed registry.
    pub const fn new(registry: &'a ConsoleRegistry) -> Self {
        Self { registry }
    }

    /// Executes one read without parsing textual syntax.
    pub fn execute(&self, plan: ReadPlan) -> ConsoleResult<ReadOutput> {
        match plan {
            ReadPlan::Schema { source } => self.registry.schema(&source).map(ReadOutput::Schema),
            ReadPlan::Get { source, arguments } => {
                if let Some(table) = self.registry.table_if_registered(&source) {
                    if arguments.len() != 1 {
                        return Err(ConsoleError::invalid_input(
                            "table get arguments",
                            format!("expected one key, got {}", arguments.len()),
                        ));
                    }
                    return table
                        .get(&arguments[0])?
                        .map(ConsoleRow::from_handle)
                        .transpose()
                        .map(ReadOutput::Record);
                }
                if let Some(view) = self.registry.view_if_registered(&source) {
                    return view
                        .get(&arguments)?
                        .map(ConsoleRow::from_handle)
                        .transpose()
                        .map(ReadOutput::Record);
                }
                Err(ConsoleError::UnknownSource(source))
            }
            ReadPlan::Scan(scan) => {
                let rows = self
                    .registry
                    .table(scan.table())?
                    .scan(scan.direction())?
                    .take(scan.limit().get())
                    .map(|record| record.and_then(ConsoleRow::from_handle));
                Ok(ReadOutput::Rows(Box::new(rows)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        ConsoleRegistry, ConsoleTable, ConsoleValue, FieldDescriptor, ScalarType, ValueMetadata,
    };

    use super::*;

    const TEST_FIELDS: &[FieldDescriptor] = &[FieldDescriptor {
        name: "value",
        scalar_type: ScalarType::U64,
        nullable: false,
    }];
    const TEST_METADATA: ValueMetadata = ValueMetadata {
        name: "TestValue",
        fields: TEST_FIELDS,
        modifiers: &[],
    };

    struct TestValue(u64);

    impl ConsoleValue for TestValue {
        fn metadata(&self) -> &'static ValueMetadata {
            &TEST_METADATA
        }

        fn get(&self, field: &str) -> ConsoleResult<ConsoleScalar> {
            match field {
                "value" => Ok(ConsoleScalar::U64(self.0)),
                _ => Err(ConsoleError::UnknownField {
                    value: TEST_METADATA.name,
                    field: field.to_owned(),
                }),
            }
        }

        fn modify(&mut self, modifier: &str, _args: &[ConsoleScalar]) -> ConsoleResult<()> {
            Err(ConsoleError::UnknownModifier {
                value: TEST_METADATA.name,
                modifier: modifier.to_owned(),
            })
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct CountingTable(Arc<AtomicUsize>);

    impl ConsoleTable for CountingTable {
        fn name(&self) -> &'static str {
            "Counting"
        }

        fn key_type(&self) -> ScalarType {
            ScalarType::U64
        }

        fn value_metadata(&self) -> &'static ValueMetadata {
            &TEST_METADATA
        }

        fn get(&self, _key: &ConsoleScalar) -> ConsoleResult<Option<RecordHandle>> {
            Ok(None)
        }

        fn scan(&self, _direction: ScanDirection) -> ConsoleResult<crate::RecordStream> {
            let pulled = self.0.clone();
            Ok(Box::new((0..5).map(move |value| {
                pulled.fetch_add(1, Ordering::SeqCst);
                Ok(RecordHandle::new(
                    "Counting",
                    ConsoleScalar::U64(value),
                    TestValue(value),
                ))
            })))
        }
    }

    #[test]
    fn bounded_scan_stops_pulling_at_limit() {
        let pulled = Arc::new(AtomicUsize::new(0));
        let mut registry = ConsoleRegistry::new();
        registry
            .register(Arc::new(CountingTable(pulled.clone())))
            .expect("test: register counting table");

        let ReadOutput::Rows(rows) = ReadExecutor::new(&registry)
            .execute(ReadPlan::scan("Counting", 2).expect("test: build scan"))
            .expect("test: execute bounded scan")
        else {
            panic!("test: scan output expected");
        };
        let rows = rows
            .collect::<ConsoleResult<Vec<_>>>()
            .expect("test: collect bounded rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(pulled.load(Ordering::SeqCst), 2);
    }
}

//! Typed read plans and their storage-independent executor.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;

use serde::Serialize;

use crate::pipeline::{PipelinePlan, execute_pipeline};
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
    pub fields: BTreeMap<String, ConsoleScalar>,
}

impl ConsoleRow {
    /// Materializes approved scalar fields from an owned value handle.
    pub fn from_handle(handle: RecordHandle) -> ConsoleResult<Self> {
        let mut fields = BTreeMap::new();
        for descriptor in handle.metadata().fields {
            fields.insert(descriptor.name.to_owned(), handle.get(descriptor.name)?);
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

/// A storage-independent console operation assembled directly in Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsolePlan {
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
    /// Executes a bounded functional table scan.
    Pipeline(PipelinePlan),
}

impl ConsolePlan {
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
        PipelinePlan::scan(table, limit).map(Self::Pipeline)
    }

    /// Creates a bounded reverse scan plan.
    pub fn scan_rev(table: impl Into<String>, limit: usize) -> ConsoleResult<Self> {
        PipelinePlan::scan_rev(table, limit).map(Self::Pipeline)
    }
}

impl From<PipelinePlan> for ConsolePlan {
    fn from(plan: PipelinePlan) -> Self {
        Self::Pipeline(plan)
    }
}

/// Lazy rows returned by a bounded scan.
pub type RowStream = Box<dyn Iterator<Item = ConsoleResult<ConsoleRow>>>;

/// Output of one typed console plan.
pub enum ConsoleOutput {
    /// Metadata for one registered source.
    Schema(SourceSchema),
    /// One point-read or native-view result.
    Row(Option<ConsoleRow>),
    /// A lazy, bounded stream of table rows.
    Rows(RowStream),
    /// A fixed scalar aggregate.
    Scalar(ConsoleScalar),
}

impl fmt::Debug for ConsoleOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(schema) => formatter.debug_tuple("Schema").field(schema).finish(),
            Self::Row(row) => formatter.debug_tuple("Row").field(row).finish(),
            Self::Rows(_) => formatter.write_str("Rows(..)"),
            Self::Scalar(value) => formatter.debug_tuple("Scalar").field(value).finish(),
        }
    }
}

/// Executes typed read plans against an explicit registry.
#[derive(Debug)]
pub struct ConsoleExecutor<'a> {
    registry: &'a ConsoleRegistry,
}

impl<'a> ConsoleExecutor<'a> {
    /// Creates an executor over a completed registry.
    pub const fn new(registry: &'a ConsoleRegistry) -> Self {
        Self { registry }
    }

    /// Executes one read without parsing textual syntax.
    pub fn execute(&self, plan: impl Into<ConsolePlan>) -> ConsoleResult<ConsoleOutput> {
        match plan.into() {
            ConsolePlan::Schema { source } => {
                self.registry.schema(&source).map(ConsoleOutput::Schema)
            }
            ConsolePlan::Get { source, arguments } => {
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
                        .map(ConsoleOutput::Row);
                }
                if let Some(view) = self.registry.view_if_registered(&source) {
                    return view
                        .get(&arguments)?
                        .map(ConsoleRow::from_handle)
                        .transpose()
                        .map(ConsoleOutput::Row);
                }
                Err(ConsoleError::UnknownSource(source))
            }
            ConsolePlan::Pipeline(plan) => execute_pipeline(self.registry, plan),
        }
    }
}

#[cfg(test)]
mod tests {
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
        settable: false,
    }];
    const TEST_METADATA: ValueMetadata = ValueMetadata {
        name: "TestValue",
        fields: TEST_FIELDS,
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

        fn set(&mut self, field: &str, _value: &ConsoleScalar) -> ConsoleResult<()> {
            Err(ConsoleError::ReadOnlyField {
                value: TEST_METADATA.name,
                field: field.to_owned(),
            })
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

        let ConsoleOutput::Rows(rows) = ConsoleExecutor::new(&registry)
            .execute(ConsolePlan::scan("Counting", 2).expect("test: build scan"))
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

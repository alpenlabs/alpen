//! Storage-independent building blocks for the Alpen database console.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde::ser::Serializer;

pub use strata_db_console_macros::{ConsoleTable, ConsoleValue};

mod expression;
mod parse;
mod pipeline;
mod read;
mod render;
mod write;

pub use expression::{BinaryOperator, ScalarExpression};
pub use parse::parse_console_plan;
pub use pipeline::{PipelinePlan, PipelineTerminal, RowSetPlan, Selection};
pub use read::{ConsoleExecutor, ConsoleOutput, ConsolePlan, ConsoleRow, RowStream, ScanPlan};
pub use render::{RecordFormat, render_record, render_schema, write_json_lines};
pub use write::{StagedWrite, WriteChange, WritePreview, WriteSession};

/// Result type returned by console registrations and values.
pub type ConsoleResult<T> = Result<T, ConsoleError>;

/// Errors surfaced by the storage-independent console layer.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// A table or alias is not registered.
    #[error("unknown console table '{0}'")]
    UnknownTable(String),

    /// A table or view is not registered.
    #[error("unknown console source '{0}'")]
    UnknownSource(String),

    /// A field is not exposed by a value registration.
    #[error("unknown field '{field}' on console value '{value}'")]
    UnknownField {
        /// Registered value name.
        value: &'static str,
        /// Requested field name.
        field: String,
    },

    /// A field is exposed for reading but not setting.
    #[error("field '{field}' on console value '{value}' is read-only")]
    ReadOnlyField {
        /// Registered value name.
        value: &'static str,
        /// Requested field name.
        field: String,
    },

    /// A modifier is not exposed by a table registration.
    #[error("unknown modifier '{modifier}' on console source '{console_source}'")]
    UnknownModifier {
        /// Registered table name.
        console_source: &'static str,
        /// Requested modifier name.
        modifier: String,
    },

    /// A key or modifier argument has the wrong scalar shape.
    #[error("invalid {target}: {message}")]
    InvalidInput {
        /// Input being parsed.
        target: &'static str,
        /// Human-readable failure detail.
        message: String,
    },

    /// A table or view name or alias was registered more than once.
    #[error("duplicate console source name or alias '{0}'")]
    DuplicateSource(String),

    /// A production storage operation failed.
    #[error("console table '{table}' storage operation failed: {source}")]
    Storage {
        /// Registered table name.
        table: &'static str,
        /// Original storage error.
        #[source]
        source: Box<dyn Error>,
    },

    /// A stable console output could not be rendered.
    #[error("failed to render console output: {0}")]
    Render(String),

    /// A write was requested for a table without a registered write path.
    #[error("console table '{0}' is read-only")]
    ReadOnlyTable(&'static str),

    /// A point or bounded-batch write targeted a record that does not exist.
    #[error("console table '{table}' has no record for the requested key")]
    MissingRecord {
        /// Registered table name.
        table: &'static str,
    },

    /// A session already contains a staged write.
    #[error("a write is already staged; commit or abort it first")]
    WriteAlreadyStaged,

    /// Commit was requested without a staged write.
    #[error("no write is staged")]
    NoStagedWrite,

    /// A stored record changed after its replacement was staged.
    #[error("staged record in console table '{table}' changed before commit")]
    StaleWrite {
        /// Registered table name.
        table: &'static str,
    },

    /// A bounded bulk modification selected no records.
    #[error("bulk modification of console table '{table}' selected no records")]
    NoMatchingRecords {
        /// Requested table name or alias.
        table: String,
    },
}

impl ConsoleError {
    /// Wraps a production storage error with its registered table name.
    pub fn storage(table: &'static str, source: impl Error + 'static) -> Self {
        Self::Storage {
            table,
            source: Box::new(source),
        }
    }

    /// Creates an invalid-input error.
    pub fn invalid_input(target: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidInput {
            target,
            message: message.into(),
        }
    }
}

/// A scalar value that may cross the Rust-to-console boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleScalar {
    /// An absent optional value.
    Null,
    /// A boolean value.
    Bool(bool),
    /// A signed integer.
    I64(i64),
    /// An unsigned integer.
    U64(u64),
    /// Human-readable text.
    String(String),
    /// Opaque bytes, including binary database keys.
    Bytes(Vec<u8>),
}

impl Serialize for ConsoleScalar {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::I64(value) => serializer.serialize_i64(*value),
            Self::U64(value) => serializer.serialize_u64(*value),
            Self::String(value) => serializer.serialize_str(value),
            Self::Bytes(value) => serializer.serialize_str(&hex::encode(value)),
        }
    }
}

impl ConsoleScalar {
    /// Returns the contained boolean or an input error.
    pub fn as_bool(&self, target: &'static str) -> ConsoleResult<bool> {
        match self {
            Self::Bool(value) => Ok(*value),
            other => Err(ConsoleError::invalid_input(
                target,
                format!("expected bool, got {}", other.kind()),
            )),
        }
    }

    /// Returns the contained signed integer or an input error.
    pub fn as_i64(&self, target: &'static str) -> ConsoleResult<i64> {
        match self {
            Self::I64(value) => Ok(*value),
            other => Err(ConsoleError::invalid_input(
                target,
                format!("expected i64, got {}", other.kind()),
            )),
        }
    }

    /// Returns the contained unsigned integer or an input error.
    pub fn as_u64(&self, target: &'static str) -> ConsoleResult<u64> {
        match self {
            Self::U64(value) => Ok(*value),
            other => Err(ConsoleError::invalid_input(
                target,
                format!("expected u64, got {}", other.kind()),
            )),
        }
    }

    /// Returns the contained bytes or an input error.
    pub fn as_bytes(&self, target: &'static str) -> ConsoleResult<&[u8]> {
        match self {
            Self::Bytes(bytes) => Ok(bytes),
            other => Err(ConsoleError::invalid_input(
                target,
                format!("expected bytes, got {}", other.kind()),
            )),
        }
    }

    /// Returns the contained string or an input error.
    pub fn as_str(&self, target: &'static str) -> ConsoleResult<&str> {
        match self {
            Self::String(value) => Ok(value),
            other => Err(ConsoleError::invalid_input(
                target,
                format!("expected string, got {}", other.kind()),
            )),
        }
    }

    /// Returns the scalar's stable type name.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::I64(_) => "i64",
            Self::U64(_) => "u64",
            Self::String(_) => "string",
            Self::Bytes(_) => "bytes",
        }
    }
}

impl fmt::Display for ConsoleScalar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Bool(value) => value.fmt(formatter),
            Self::I64(value) => value.fmt(formatter),
            Self::U64(value) => value.fmt(formatter),
            Self::String(value) => formatter.write_str(value),
            Self::Bytes(value) => formatter.write_str(&hex::encode(value)),
        }
    }
}

/// Scalar type metadata exposed by a getter or modifier argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    /// Boolean values.
    Bool,
    /// Signed integers.
    I64,
    /// Unsigned integers.
    U64,
    /// Strings.
    String,
    /// Byte strings.
    Bytes,
}

impl ScalarType {
    /// Returns the stable schema name for this scalar type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::String => "string",
            Self::Bytes => "bytes",
        }
    }
}

/// Metadata for an approved getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FieldDescriptor {
    /// Getter name visible to the console.
    pub name: &'static str,
    /// Scalar type returned by the getter.
    pub scalar_type: ScalarType,
    /// Whether the getter may return [`ConsoleScalar::Null`].
    pub nullable: bool,
    /// Whether the field has an approved domain setter.
    pub settable: bool,
}

/// Metadata for a modifier argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ArgumentDescriptor {
    /// Argument name shown in source metadata and help.
    pub name: &'static str,
    /// Required scalar type.
    pub scalar_type: ScalarType,
}

/// Metadata for an approved modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ModifierDescriptor {
    /// Modifier name visible to the console.
    pub name: &'static str,
    /// Ordered modifier arguments.
    pub arguments: &'static [ArgumentDescriptor],
}

/// Static metadata generated or implemented for a concrete Rust value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ValueMetadata {
    /// Human-readable value type name.
    pub name: &'static str,
    /// Approved getters.
    pub fields: &'static [FieldDescriptor],
}

/// Type-erased operations approved for a concrete Rust value.
pub trait ConsoleValue: Send + Sync {
    /// Returns the value's static console metadata.
    fn metadata(&self) -> &'static ValueMetadata;

    /// Invokes an approved getter.
    fn get(&self, field: &str) -> ConsoleResult<ConsoleScalar>;

    /// Invokes an approved domain setter on the real Rust value.
    fn set(&mut self, field: &str, value: &ConsoleScalar) -> ConsoleResult<()>;
}

/// Companion trait that exposes metadata without constructing a value.
pub trait RegisteredConsoleValue: ConsoleValue {
    /// Returns the type's static console metadata.
    fn value_metadata() -> &'static ValueMetadata;
}

/// An owned concrete database value with its table and key identity.
pub struct RecordHandle {
    table: &'static str,
    key: ConsoleScalar,
    value: Box<dyn ConsoleValue>,
}

impl RecordHandle {
    /// Creates a handle from an owned concrete value.
    pub fn new(
        table: &'static str,
        key: ConsoleScalar,
        value: impl ConsoleValue + 'static,
    ) -> Self {
        Self {
            table,
            key,
            value: Box::new(value),
        }
    }

    /// Returns the table that produced this handle.
    pub const fn table(&self) -> &'static str {
        self.table
    }

    /// Returns the scalar form of the typed database key.
    pub fn key(&self) -> &ConsoleScalar {
        &self.key
    }

    /// Returns the approved metadata for the held value.
    pub fn metadata(&self) -> &'static ValueMetadata {
        self.value.metadata()
    }

    /// Invokes an approved getter on the held value.
    pub fn get(&self, field: &str) -> ConsoleResult<ConsoleScalar> {
        self.value.get(field)
    }
}

impl fmt::Debug for RecordHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordHandle")
            .field("table", &self.table)
            .field("key", &self.key)
            .field("value", &self.value.metadata().name)
            .finish()
    }
}

/// Direction used when traversing a registered table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanDirection {
    /// Traverse in encoded key order.
    #[default]
    Forward,
    /// Traverse in reverse encoded key order.
    Reverse,
}

/// Owned, lazy stream of decoded record handles.
pub type RecordStream = Box<dyn Iterator<Item = ConsoleResult<RecordHandle>>>;

/// Type-erased access to one concrete database table.
pub trait ConsoleTable: Send + Sync {
    /// Stable table name used by console programs.
    fn name(&self) -> &'static str;

    /// Optional alternate names.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Scalar type accepted by this table's key parser.
    fn key_type(&self) -> ScalarType;

    /// Metadata for values returned by this table.
    fn value_metadata(&self) -> &'static ValueMetadata;

    /// Broader domain operations approved for this table.
    fn modifiers(&self) -> &'static [ModifierDescriptor] {
        &[]
    }

    /// Reads one record using a scalar key.
    fn get(&self, key: &ConsoleScalar) -> ConsoleResult<Option<RecordHandle>>;

    /// Lazily scans the table in encoded key order.
    fn scan(&self, direction: ScanDirection) -> ConsoleResult<RecordStream>;

    /// Prepares a point field update without persisting it.
    fn stage_set(
        &self,
        _key: &ConsoleScalar,
        _field: &str,
        _value: &ConsoleScalar,
    ) -> ConsoleResult<Box<dyn StagedWrite>> {
        Err(ConsoleError::ReadOnlyTable(self.name()))
    }

    /// Prepares a broader point modification without persisting it.
    fn stage_modify(
        &self,
        _key: &ConsoleScalar,
        _modifier: &str,
        _arguments: &[ConsoleScalar],
    ) -> ConsoleResult<Box<dyn StagedWrite>> {
        Err(ConsoleError::ReadOnlyTable(self.name()))
    }

    /// Prepares one broader operation across a bounded set of point keys.
    fn stage_modify_many(
        &self,
        _keys: &[ConsoleScalar],
        _modifier: &str,
        _arguments: &[ConsoleScalar],
    ) -> ConsoleResult<Box<dyn StagedWrite>> {
        Err(ConsoleError::ReadOnlyTable(self.name()))
    }
}

/// A trusted Rust read that may combine multiple database tables.
pub trait ConsoleView: Send + Sync {
    /// Stable view name used by console programs.
    fn name(&self) -> &'static str;

    /// Optional alternate names.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Ordered scalar arguments accepted by the view.
    fn arguments(&self) -> &'static [ArgumentDescriptor];

    /// Metadata for values returned by this view.
    fn value_metadata(&self) -> &'static ValueMetadata;

    /// Executes the trusted read with checked scalar arguments.
    fn get(&self, arguments: &[ConsoleScalar]) -> ConsoleResult<Option<RecordHandle>>;
}

/// Kind of registered read source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A directly registered storage table.
    Table,
    /// A trusted Rust read across one or more tables.
    View,
}

impl SourceKind {
    /// Returns the stable schema name for this source kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
        }
    }
}

/// Schema returned for a table or native view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SourceSchema {
    /// Primary source name.
    pub name: &'static str,
    /// Alternate names accepted by the registry.
    pub aliases: &'static [&'static str],
    /// Whether the source is a table or native view.
    pub kind: SourceKind,
    /// Table key type, absent for a native view.
    pub key_type: Option<ScalarType>,
    /// Ordered native-view arguments, empty for a table.
    pub arguments: &'static [ArgumentDescriptor],
    /// Metadata for the returned value.
    pub value: &'static ValueMetadata,
    /// Broader domain operations, empty for views and read-only tables.
    pub modifiers: &'static [ModifierDescriptor],
}

/// Explicit registry of sources supported by one console instance.
#[derive(Default)]
pub struct ConsoleRegistry {
    sources: BTreeMap<&'static str, RegisteredSource>,
}

#[derive(Clone)]
enum RegisteredSource {
    Table(Arc<dyn ConsoleTable>),
    View(Arc<dyn ConsoleView>),
}

impl RegisteredSource {
    fn schema(&self) -> SourceSchema {
        match self {
            Self::Table(table) => SourceSchema {
                name: table.name(),
                aliases: table.aliases(),
                kind: SourceKind::Table,
                key_type: Some(table.key_type()),
                arguments: &[],
                value: table.value_metadata(),
                modifiers: table.modifiers(),
            },
            Self::View(view) => SourceSchema {
                name: view.name(),
                aliases: view.aliases(),
                kind: SourceKind::View,
                key_type: None,
                arguments: view.arguments(),
                value: view.value_metadata(),
                modifiers: &[],
            },
        }
    }
}

impl ConsoleRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a table and its aliases.
    pub fn register(&mut self, table: Arc<dyn ConsoleTable>) -> ConsoleResult<()> {
        let name = table.name();
        let aliases = table.aliases();
        self.register_source(name, aliases, RegisteredSource::Table(table))
    }

    /// Registers a native view and its aliases.
    pub fn register_view(&mut self, view: Arc<dyn ConsoleView>) -> ConsoleResult<()> {
        let name = view.name();
        let aliases = view.aliases();
        self.register_source(name, aliases, RegisteredSource::View(view))
    }

    fn register_source(
        &mut self,
        name: &'static str,
        aliases: &'static [&'static str],
        source: RegisteredSource,
    ) -> ConsoleResult<()> {
        let mut names = Vec::with_capacity(aliases.len() + 1);
        names.push(name);
        names.extend_from_slice(aliases);

        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name) || self.sources.contains_key(name) {
                return Err(ConsoleError::DuplicateSource((*name).to_owned()));
            }
        }

        for name in names {
            self.sources.insert(name, source.clone());
        }
        Ok(())
    }

    /// Returns a table by its primary name or alias.
    pub fn table(&self, name: &str) -> ConsoleResult<&Arc<dyn ConsoleTable>> {
        match self.sources.get(name) {
            Some(RegisteredSource::Table(table)) => Ok(table),
            Some(RegisteredSource::View(_)) | None => {
                Err(ConsoleError::UnknownTable(name.to_owned()))
            }
        }
    }

    /// Returns the schema for a table or native view.
    pub fn schema(&self, name: &str) -> ConsoleResult<SourceSchema> {
        self.sources
            .get(name)
            .map(RegisteredSource::schema)
            .ok_or_else(|| ConsoleError::UnknownSource(name.to_owned()))
    }

    /// Iterates over registered names, including aliases.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.sources.keys().copied()
    }

    pub(crate) fn table_if_registered(&self, name: &str) -> Option<&Arc<dyn ConsoleTable>> {
        match self.sources.get(name) {
            Some(RegisteredSource::Table(table)) => Some(table),
            Some(RegisteredSource::View(_)) | None => None,
        }
    }

    pub(crate) fn view_if_registered(&self, name: &str) -> Option<&Arc<dyn ConsoleView>> {
        match self.sources.get(name) {
            Some(RegisteredSource::View(view)) => Some(view),
            Some(RegisteredSource::Table(_)) | None => None,
        }
    }
}

impl fmt::Debug for ConsoleRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleRegistry")
            .field("sources", &self.sources.keys())
            .finish()
    }
}

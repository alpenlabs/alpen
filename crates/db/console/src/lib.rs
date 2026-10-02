//! Storage-independent building blocks for the Alpen database console.

use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde::ser::Serializer;

pub use strata_db_console_macros::{ConsoleTable, ConsoleValue};

mod read;
mod render;

pub use read::{ConsoleRow, ReadExecutor, ReadOutput, ReadPlan, RowStream, ScanPlan};
pub use render::{RecordFormat, render_record, render_schema, write_json_lines};

/// Result type returned by console registrations and values.
pub type ConsoleResult<T> = Result<T, ConsoleError>;

/// Errors surfaced by the storage-independent console layer.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// A table or alias is not registered.
    #[error("unknown console table '{0}'")]
    UnknownTable(String),

    /// A view or alias is not registered.
    #[error("unknown console view '{0}'")]
    UnknownView(String),

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

    /// A modifier is not exposed by a value registration.
    #[error("unknown modifier '{modifier}' on console value '{value}'")]
    UnknownModifier {
        /// Registered value name.
        value: &'static str,
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

    /// Registered read data was internally incomplete or inconsistent.
    #[error("console source '{read_source}' read failed: {message}")]
    Read {
        /// Primary source name.
        read_source: &'static str,
        /// Description of the missing or inconsistent data.
        message: String,
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

    /// Creates an internally inconsistent read error.
    pub fn read(source: &'static str, message: impl Into<String>) -> Self {
        Self::Read {
            read_source: source,
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
}

/// Metadata for a modifier argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ArgumentDescriptor {
    /// Argument name used by help and future named-argument support.
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
    /// Approved modifiers.
    pub modifiers: &'static [ModifierDescriptor],
}

/// Type-erased operations approved for a concrete Rust value.
pub trait ConsoleValue: Any + Send + Sync {
    /// Returns the value's static console metadata.
    fn metadata(&self) -> &'static ValueMetadata;

    /// Invokes an approved getter.
    fn get(&self, field: &str) -> ConsoleResult<ConsoleScalar>;

    /// Invokes an approved modifier on the real Rust value.
    fn modify(&mut self, modifier: &str, args: &[ConsoleScalar]) -> ConsoleResult<()>;

    /// Returns the concrete value as [`Any`] for typed storage adapters.
    fn as_any(&self) -> &dyn Any;

    /// Returns the mutable concrete value as [`Any`] for typed storage adapters.
    fn as_any_mut(&mut self) -> &mut dyn Any;
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

    /// Invokes an approved modifier on the held value.
    pub fn modify(&mut self, modifier: &str, args: &[ConsoleScalar]) -> ConsoleResult<()> {
        self.value.modify(modifier, args)
    }

    /// Borrows the held value as its concrete type for a storage adapter.
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.value.as_any().downcast_ref()
    }

    /// Mutably borrows the held value as its concrete type for a storage adapter.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.value.as_any_mut().downcast_mut()
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

    /// Reads one record using a scalar key.
    fn get(&self, key: &ConsoleScalar) -> ConsoleResult<Option<RecordHandle>>;

    /// Lazily scans the table in encoded key order.
    fn scan(&self, direction: ScanDirection) -> ConsoleResult<RecordStream>;
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
}

/// Explicit registry of tables supported by one console instance.
#[derive(Default)]
pub struct ConsoleRegistry {
    tables: BTreeMap<&'static str, Arc<dyn ConsoleTable>>,
    views: BTreeMap<&'static str, Arc<dyn ConsoleView>>,
}

impl ConsoleRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a table and its aliases.
    pub fn register(&mut self, table: Arc<dyn ConsoleTable>) -> ConsoleResult<()> {
        let mut names = Vec::with_capacity(table.aliases().len() + 1);
        names.push(table.name());
        names.extend_from_slice(table.aliases());

        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name)
                || self.tables.contains_key(name)
                || self.views.contains_key(name)
            {
                return Err(ConsoleError::DuplicateSource((*name).to_owned()));
            }
        }

        for name in names {
            self.tables.insert(name, table.clone());
        }
        Ok(())
    }

    /// Registers a native view and its aliases.
    pub fn register_view(&mut self, view: Arc<dyn ConsoleView>) -> ConsoleResult<()> {
        let mut names = Vec::with_capacity(view.aliases().len() + 1);
        names.push(view.name());
        names.extend_from_slice(view.aliases());

        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name)
                || self.tables.contains_key(name)
                || self.views.contains_key(name)
            {
                return Err(ConsoleError::DuplicateSource((*name).to_owned()));
            }
        }

        for name in names {
            self.views.insert(name, view.clone());
        }
        Ok(())
    }

    /// Returns a table by its primary name or alias.
    pub fn table(&self, name: &str) -> ConsoleResult<&Arc<dyn ConsoleTable>> {
        self.tables
            .get(name)
            .ok_or_else(|| ConsoleError::UnknownTable(name.to_owned()))
    }

    /// Returns a native view by its primary name or alias.
    pub fn view(&self, name: &str) -> ConsoleResult<&Arc<dyn ConsoleView>> {
        self.views
            .get(name)
            .ok_or_else(|| ConsoleError::UnknownView(name.to_owned()))
    }

    /// Returns the schema for a table or native view.
    pub fn schema(&self, name: &str) -> ConsoleResult<SourceSchema> {
        if let Some(table) = self.tables.get(name) {
            return Ok(SourceSchema {
                name: table.name(),
                aliases: table.aliases(),
                kind: SourceKind::Table,
                key_type: Some(table.key_type()),
                arguments: &[],
                value: table.value_metadata(),
            });
        }
        if let Some(view) = self.views.get(name) {
            return Ok(SourceSchema {
                name: view.name(),
                aliases: view.aliases(),
                kind: SourceKind::View,
                key_type: None,
                arguments: view.arguments(),
                value: view.value_metadata(),
            });
        }
        Err(ConsoleError::UnknownSource(name.to_owned()))
    }

    /// Iterates over registered names, including aliases.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.tables.keys().chain(self.views.keys()).copied()
    }

    pub(crate) fn table_if_registered(&self, name: &str) -> Option<&Arc<dyn ConsoleTable>> {
        self.tables.get(name)
    }

    pub(crate) fn view_if_registered(&self, name: &str) -> Option<&Arc<dyn ConsoleView>> {
        self.views.get(name)
    }
}

impl fmt::Debug for ConsoleRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleRegistry")
            .field("tables", &self.tables.keys())
            .field("views", &self.views.keys())
            .finish()
    }
}

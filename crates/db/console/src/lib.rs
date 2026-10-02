//! Storage-independent building blocks for the Alpen database console.

use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

pub use strata_db_console_macros::{ConsoleTable, ConsoleValue};

/// Result type returned by console registrations and values.
pub type ConsoleResult<T> = Result<T, ConsoleError>;

/// Errors surfaced by the storage-independent console layer.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// A table or alias is not registered.
    #[error("unknown console table '{0}'")]
    UnknownTable(String),

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

    /// A table name or alias was registered more than once.
    #[error("duplicate console table name or alias '{0}'")]
    DuplicateTable(String),

    /// A production storage operation failed.
    #[error("console table '{table}' storage operation failed: {source}")]
    Storage {
        /// Registered table name.
        table: &'static str,
        /// Original storage error.
        #[source]
        source: Box<dyn Error>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Metadata for an approved getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// Getter name visible to the console.
    pub name: &'static str,
    /// Scalar type returned by the getter.
    pub scalar_type: ScalarType,
    /// Whether the getter may return [`ConsoleScalar::Null`].
    pub nullable: bool,
}

/// Metadata for a modifier argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentDescriptor {
    /// Argument name used by help and future named-argument support.
    pub name: &'static str,
    /// Required scalar type.
    pub scalar_type: ScalarType,
}

/// Metadata for an approved modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierDescriptor {
    /// Modifier name visible to the console.
    pub name: &'static str,
    /// Ordered modifier arguments.
    pub arguments: &'static [ArgumentDescriptor],
}

/// Static metadata generated or implemented for a concrete Rust value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Metadata for values returned by this table.
    fn value_metadata(&self) -> &'static ValueMetadata;

    /// Reads one record using a scalar key.
    fn get(&self, key: &ConsoleScalar) -> ConsoleResult<Option<RecordHandle>>;

    /// Lazily scans the table in encoded key order.
    fn scan(&self, direction: ScanDirection) -> ConsoleResult<RecordStream>;
}

/// Explicit registry of tables supported by one console instance.
#[derive(Default)]
pub struct ConsoleRegistry {
    tables: BTreeMap<&'static str, Arc<dyn ConsoleTable>>,
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
            if names[..index].contains(name) || self.tables.contains_key(name) {
                return Err(ConsoleError::DuplicateTable((*name).to_owned()));
            }
        }

        for name in names {
            self.tables.insert(name, table.clone());
        }
        Ok(())
    }

    /// Returns a table by its primary name or alias.
    pub fn table(&self, name: &str) -> ConsoleResult<&Arc<dyn ConsoleTable>> {
        self.tables
            .get(name)
            .ok_or_else(|| ConsoleError::UnknownTable(name.to_owned()))
    }

    /// Iterates over registered names, including aliases.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.tables.keys().copied()
    }
}

impl fmt::Debug for ConsoleRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleRegistry")
            .field("names", &self.tables.keys())
            .finish()
    }
}

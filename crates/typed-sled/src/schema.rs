use std::fmt::Debug;

use crate::codec::{KeyCodec, ValueCodec};

/// A type-safe wrapper for tree names.
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct TreeName(pub &'static str);

impl TreeName {
    /// Extracts the inner string slice.
    pub fn into_inner(self) -> &'static str {
        self.0
    }
}

impl From<&'static str> for TreeName {
    fn from(value: &'static str) -> Self {
        Self(value)
    }
}

/// Defines the schema for a typed tree with associated key and value types.
pub trait Schema: Debug + Send + Sync + Sized {
    const TREE_NAME: TreeName;

    type Key: KeyCodec<Self>;
    type Value: ValueCodec<Self>;
}

//! Shared test utilities for typed-sled crate.
//!
//! This module provides common test infrastructure that can be reused across
//! all test modules, eliminating code duplication and ensuring consistency.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{CodecError, CodecResult, Schema, SledDb, TreeName, ValueCodec};

/// Common test value type used across all tests.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub(crate) struct TestValue {
    pub id: u32,
    pub name: String,
}

impl TestValue {
    /// Creates a test value for "Alice"
    pub(crate) fn alice() -> Self {
        Self {
            id: 1,
            name: "Alice".to_string(),
        }
    }

    /// Creates a test value for "Bob"
    pub(crate) fn bob() -> Self {
        Self {
            id: 2,
            name: "Bob".to_string(),
        }
    }

    /// Creates a test value for "Charlie"
    pub(crate) fn charlie() -> Self {
        Self {
            id: 3,
            name: "Charlie".to_string(),
        }
    }

    /// Creates a test value with a generated name based on id
    pub(crate) fn new_with_name(id: u32) -> Self {
        Self {
            id,
            name: format!("Item {}", id),
        }
    }

    /// Creates a test value with custom id and name
    pub(crate) fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
        }
    }
}

/// Test schema 1 - uses "test1" as tree name
#[derive(Debug, Clone)]
pub(crate) struct TestSchema1;

impl Schema for TestSchema1 {
    const TREE_NAME: TreeName = TreeName("test1");
    type Key = u32;
    type Value = TestValue;
}

/// Test schema 2 - uses "test2" as tree name
#[derive(Debug, Clone)]
pub(crate) struct TestSchema2;

impl Schema for TestSchema2 {
    const TREE_NAME: TreeName = TreeName("test2");
    type Key = u32;
    type Value = TestValue;
}

/// Test schema 3 - uses "test3" as tree name
#[derive(Debug, Clone)]
pub(crate) struct TestSchema3;

impl Schema for TestSchema3 {
    const TREE_NAME: TreeName = TreeName("test3");
    type Key = u32;
    type Value = TestValue;
}

impl<S> ValueCodec<S> for TestValue
where
    S: Schema<Key = u32, Value = TestValue>,
{
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        borsh::to_vec(self).map_err(|e| CodecError::SerializationFailed {
            schema: S::TREE_NAME.0,
            source: e.into(),
        })
    }

    fn decode_value(buf: &[u8]) -> CodecResult<Self> {
        borsh::from_slice(buf).map_err(|e| CodecError::DeserializationFailed {
            schema: S::TREE_NAME.0,
            source: e.into(),
        })
    }
}

/// Creates a temporary sled database for testing
pub(crate) fn create_temp_sled_db() -> sled::Db {
    sled::Config::new().temporary(true).open().unwrap()
}

/// Creates a typed sled database wrapper for testing
pub(crate) fn create_test_db() -> crate::error::Result<SledDb> {
    let sled_db = create_temp_sled_db();
    SledDb::new(sled_db)
}

/// Creates a temporary tree for direct tree testing (bypassing SledDb wrapper)
pub(crate) fn create_temp_tree<S: Schema>() -> crate::error::Result<crate::SledTree<S>> {
    let sled_db = create_temp_sled_db();
    let tree = sled_db.open_tree(S::TREE_NAME.into_inner())?;
    Ok(crate::SledTree::new(tree))
}

/// Helper for asserting two TestValues are equal with better error messages
#[track_caller]
pub(crate) fn assert_test_values_eq(expected: &TestValue, actual: &TestValue) {
    assert_eq!(
        expected, actual,
        "TestValues differ: expected id={}, name='{}' but got id={}, name='{}'",
        expected.id, expected.name, actual.id, actual.name
    );
}

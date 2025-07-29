use std::{
    marker::PhantomData,
    ops::{Bound, RangeBounds},
    sync::Arc,
};

use sled::{IVec, Iter, Tree, transaction::TransactionalTree};

use crate::{KeyCodec, Schema, ValueCodec, batch::SledBatch, error::Result};

/// Decodes a raw key-value pair into typed schema types.
fn decode_pair<S: Schema>((k, v): (IVec, IVec)) -> Result<(S::Key, S::Value)> {
    let key = S::Key::decode_key(&k)?;
    let value = S::Value::decode_value(&v)?;
    Ok((key, value))
}

/// Converts a typed key bound to a raw byte bound.
fn key_bound<S: Schema>(k: Bound<&S::Key>) -> Result<Bound<Vec<u8>>> {
    let bound = match k {
        Bound::Included(k) => Bound::Included(k.encode_key()?),
        Bound::Excluded(k) => Bound::Excluded(k.encode_key()?),
        Bound::Unbounded => Bound::Unbounded,
    };
    Ok(bound)
}

/// Type-safe wrapper around a sled tree with schema-enforced operations.
#[derive(Debug)]
pub struct SledTree<S: Schema> {
    pub(crate) inner: Arc<Tree>,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledTree<S> {
    /// Creates a new typed tree wrapper.
    pub fn new(inner: Arc<Tree>) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }

    /// Inserts a key-value pair into the tree.
    pub fn insert(&self, key: &S::Key, value: &S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value)?;

        self.inner.flush()?;
        Ok(())
    }

    /// Retrieves a value for the given key.
    pub fn get(&self, key: &S::Key) -> Result<Option<S::Value>> {
        let key = key.encode_key()?;
        let val = self.inner.get(key)?;
        let val = val.as_deref();
        Ok(val.map(|v| S::Value::decode_value(v)).transpose()?)
    }

    /// Removes a key-value pair from the tree.
    pub fn remove(&self, key: &S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key)?;

        self.inner.flush()?;
        Ok(())
    }

    /// Returns true if the tree contains no key-value pairs.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the first key-value pair in the tree.
    pub fn first(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.first()?.map(decode_pair::<S>).transpose()
    }

    /// Returns the last key-value pair in the tree.
    pub fn last(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.last()?.map(decode_pair::<S>).transpose()
    }

    /// Applies a batch of operations atomically.
    pub fn apply_batch(&self, batch: SledBatch<S>) -> Result<()> {
        Ok(self.inner.apply_batch(batch.inner)?)
    }

    /// Returns an iterator over all key-value pairs in the tree.
    pub fn iter(&self) -> SledTreeIter<S> {
        SledTreeIter {
            inner: self.inner.iter(),
            _phantom: PhantomData,
        }
    }

    /// Returns an iterator over key-value pairs within the specified range.
    pub fn range<R>(&self, range: R) -> Result<SledTreeIter<S>>
    where
        R: RangeBounds<S::Key>,
    {
        let start = key_bound::<S>(range.start_bound())?;
        let end = key_bound::<S>(range.end_bound())?;
        Ok(SledTreeIter {
            inner: self.inner.range((start, end)),
            _phantom: PhantomData,
        })
    }
}

/// Type-safe wrapper around sled's transactional tree.
pub struct SledTransactionalTree<S: Schema> {
    inner: TransactionalTree,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledTransactionalTree<S> {
    /// Creates a new transactional tree wrapper.
    pub fn new(inner: TransactionalTree) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }

    /// Inserts a key-value pair in the transaction.
    pub fn insert(&self, key: &S::Key, value: &S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value)?;
        Ok(())
    }

    /// Retrieves a value for the given key within the transaction.
    pub fn get(&self, key: &S::Key) -> Result<Option<S::Value>> {
        let key = key.encode_key()?;
        let val = self.inner.get(key)?;
        let val = val.as_deref();
        Ok(val.map(|v| S::Value::decode_value(v)).transpose()?)
    }

    /// Removes a key-value pair within the transaction.
    pub fn remove(&self, key: &S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key)?;
        Ok(())
    }
}

/// A typed iterator over key-value pairs in a sled tree.
pub struct SledTreeIter<S: Schema> {
    inner: Iter,
    _phantom: PhantomData<S>,
}

impl<S: Schema> Iterator for SledTreeIter<S> {
    type Item = Result<(S::Key, S::Value)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|result| result.map_err(Into::into).and_then(decode_pair::<S>))
    }
}

impl<S: Schema> DoubleEndedIterator for SledTreeIter<S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|result| result.map_err(Into::into).and_then(decode_pair::<S>))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use borsh::{BorshDeserialize, BorshSerialize};

    use super::*;
    use crate::{CodecError, CodecResult, Schema, TreeName};

    #[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
    struct TestValue {
        id: u32,
        name: String,
    }

    impl TestValue {
        pub fn new_with_name(id: u32) -> Self {
            Self {
                id,
                name: format!("Item {id}"),
            }
        }
    }

    #[derive(Debug)]
    struct TestSchema;

    impl Schema for TestSchema {
        const TREE_NAME: TreeName = TreeName("test");
        type Key = u32;
        type Value = TestValue;
    }

    impl KeyCodec<TestSchema> for u32 {
        fn encode_key(&self) -> CodecResult<Vec<u8>> {
            Ok(self.to_be_bytes().to_vec())
        }

        fn decode_key(buf: &[u8]) -> CodecResult<Self> {
            if buf.len() != 4 {
                return Err(CodecError::InvalidKeyLength {
                    schema: TestSchema::TREE_NAME.0,
                    expected: 4,
                    actual: buf.len(),
                });
            }
            let mut bytes = [0; 4];
            bytes.copy_from_slice(buf);
            Ok(u32::from_be_bytes(bytes))
        }
    }

    impl ValueCodec<TestSchema> for TestValue {
        fn encode_value(&self) -> CodecResult<Vec<u8>> {
            borsh::to_vec(self).map_err(|e| CodecError::SerializationFailed {
                schema: TestSchema::TREE_NAME.0,
                source: e.into(),
            })
        }
        fn decode_value(buf: &[u8]) -> CodecResult<Self> {
            borsh::from_slice(buf).map_err(|e| CodecError::DeserializationFailed {
                schema: TestSchema::TREE_NAME.0,
                source: e.into(),
            })
        }
    }

    fn create_test_tree() -> Result<SledTree<TestSchema>> {
        let sled_db = sled::Config::new().temporary(true).open().unwrap();
        let tree = Arc::new(sled_db.open_tree("test").unwrap());
        Ok(SledTree::new(tree))
    }

    #[test]
    fn test_iter_empty() {
        let tree = create_test_tree().unwrap();
        let mut iter = tree.iter();
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_forward() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        tree.insert(
            &1,
            &TestValue {
                id: 1,
                name: "Alice".to_string(),
            },
        )
        .unwrap();
        tree.insert(
            &3,
            &TestValue {
                id: 3,
                name: "Charlie".to_string(),
            },
        )
        .unwrap();
        tree.insert(
            &2,
            &TestValue {
                id: 2,
                name: "Bob".to_string(),
            },
        )
        .unwrap();

        let items: Result<Vec<_>> = tree.iter().collect();
        let items = items.unwrap();

        // Should be sorted by key
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 1);
        assert_eq!(items[1].0, 2);
        assert_eq!(items[2].0, 3);
        assert_eq!(items[0].1.name, "Alice");
        assert_eq!(items[1].1.name, "Bob");
        assert_eq!(items[2].1.name, "Charlie");
    }

    #[test]
    fn test_iter_backward() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        tree.insert(
            &1,
            &TestValue {
                id: 1,
                name: "Alice".to_string(),
            },
        )
        .unwrap();
        tree.insert(
            &3,
            &TestValue {
                id: 3,
                name: "Charlie".to_string(),
            },
        )
        .unwrap();
        tree.insert(
            &2,
            &TestValue {
                id: 2,
                name: "Bob".to_string(),
            },
        )
        .unwrap();

        let items: Result<Vec<_>> = tree.iter().rev().collect();
        let items = items.unwrap();
        println!("{items:?}");

        // Should be reverse sorted by key
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 3);
        assert_eq!(items[1].0, 2);
        assert_eq!(items[2].0, 1);
        assert_eq!(items[0].1.name, "Charlie");
        assert_eq!(items[1].1.name, "Bob");
        assert_eq!(items[2].1.name, "Alice");
    }

    #[test]
    fn test_range_inclusive() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        for i in 1..=5 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        let items: Result<Vec<_>> = tree.range(2..=4).unwrap().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 2);
        assert_eq!(items[1].0, 3);
        assert_eq!(items[2].0, 4);
    }

    #[test]
    fn test_range_exclusive() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        for i in 1..=5 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        let items: Result<Vec<_>> = tree.range(2..4).unwrap().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, 2);
        assert_eq!(items[1].0, 3);
    }

    #[test]
    fn test_range_from() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        for i in 1..=5 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        let items: Result<Vec<_>> = tree.range(3..).unwrap().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 3);
        assert_eq!(items[1].0, 4);
        assert_eq!(items[2].0, 5);
    }

    #[test]
    fn test_range_to() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        for i in 1..=5 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        let items: Result<Vec<_>> = tree.range(..=3).unwrap().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 1);
        assert_eq!(items[1].0, 2);
        assert_eq!(items[2].0, 3);
    }

    #[test]
    fn test_range_double_ended() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        for i in 1..=5 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        let items: Result<Vec<_>> = tree.range(2..=4).unwrap().rev().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 4);
        assert_eq!(items[1].0, 3);
        assert_eq!(items[2].0, 2);
    }

    #[test]
    fn test_u32_key_ordering_large_values() {
        let tree = create_test_tree().unwrap();

        // Insert keys > 256 to test proper u32 ordering
        let keys = [100, 255, 256, 300, 500];

        for &key in &keys {
            tree.insert(&key, &TestValue::new_with_name(key)).unwrap();
        }

        // Test forward iteration - should be numerically ordered
        let items: Result<Vec<_>> = tree.iter().collect();
        let items = items.unwrap();

        assert_eq!(items.len(), 5);
        assert_eq!(items[0].0, 100);
        assert_eq!(items[1].0, 255);
        assert_eq!(items[2].0, 256);
        assert_eq!(items[3].0, 300);
        assert_eq!(items[4].0, 500);

        // Test range query with values > 256
        let range_items: Result<Vec<_>> = tree.range(256..=400).unwrap().collect();
        let range_items = range_items.unwrap();

        assert_eq!(range_items.len(), 2);
        assert_eq!(range_items[0].0, 256);
        assert_eq!(range_items[1].0, 300);
    }
}

use std::{
    marker::PhantomData,
    ops::{Bound, RangeBounds},
    sync::Arc,
};

use sled::{IVec, Iter, Tree};

use crate::{KeyCodec, Schema, ValueCodec, batch::SledBatch, error::Result};

pub struct SledTree<S: Schema> {
    inner: Arc<Tree>,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledTree<S> {
    pub fn new(inner: Arc<Tree>) -> Self {
        Self {
            inner,
            _phantom: PhantomData,
        }
    }

    pub fn put(&self, key: &S::Key, value: &S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value)?;

        self.inner.flush()?;
        Ok(())
    }

    pub fn get(&self, key: &S::Key) -> Result<Option<S::Value>> {
        let key = key.encode_key()?;
        let val = self.inner.get(key)?;
        let val = val.as_deref();
        Ok(val.map(|v| S::Value::decode_value(v)).transpose()?)
    }

    pub fn remove(&self, key: &S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key)?;

        self.inner.flush()?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn first(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.first()?.map(Self::decode_pair).transpose()
    }

    pub fn last(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.last()?.map(Self::decode_pair).transpose()
    }

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
        let start = Self::key_bound(range.start_bound())?;
        let end = Self::key_bound(range.end_bound())?;
        Ok(SledTreeIter {
            inner: self.inner.range((start, end)),
            _phantom: PhantomData,
        })
    }

    fn decode_pair((k, v): (IVec, IVec)) -> Result<(S::Key, S::Value)> {
        println!("{k:?}");
        let key = S::Key::decode_key(&k)?;
        let value = S::Value::decode_value(&v)?;
        Ok((key, value))
    }

    fn key_bound(k: Bound<&S::Key>) -> Result<Bound<Vec<u8>>> {
        let bound = match k {
            Bound::Included(k) => Bound::Included(k.encode_key()?),
            Bound::Excluded(k) => Bound::Excluded(k.encode_key()?),
            Bound::Unbounded => Bound::Unbounded,
        };
        Ok(bound)
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
        self.inner.next().map(|result| {
            result
                .map_err(Into::into)
                .and_then(SledTree::<S>::decode_pair)
        })
    }
}

impl<S: Schema> DoubleEndedIterator for SledTreeIter<S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|result| {
            result
                .map_err(Into::into)
                .and_then(SledTree::<S>::decode_pair)
        })
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
                return Err(CodecError::InvalidLength {
                    expected: 4,
                    got: buf.len(),
                });
            }
            let mut bytes = [0; 4];
            bytes.copy_from_slice(buf);
            Ok(u32::from_be_bytes(bytes))
        }
    }

    impl ValueCodec<TestSchema> for TestValue {
        fn encode_value(&self) -> CodecResult<Vec<u8>> {
            borsh::to_vec(self).map_err(CodecError::Serialization)
        }
        fn decode_value(buf: &[u8]) -> CodecResult<Self> {
            borsh::from_slice(buf).map_err(CodecError::Deserialization)
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
        tree.put(
            &1,
            &TestValue {
                id: 1,
                name: "Alice".to_string(),
            },
        )
        .unwrap();
        tree.put(
            &3,
            &TestValue {
                id: 3,
                name: "Charlie".to_string(),
            },
        )
        .unwrap();
        tree.put(
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
        tree.put(
            &1,
            &TestValue {
                id: 1,
                name: "Alice".to_string(),
            },
        )
        .unwrap();
        tree.put(
            &3,
            &TestValue {
                id: 3,
                name: "Charlie".to_string(),
            },
        )
        .unwrap();
        tree.put(
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
            tree.put(&i, &TestValue::new_with_name(i)).unwrap();
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
            tree.put(&i, &TestValue::new_with_name(i)).unwrap();
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
            tree.put(&i, &TestValue::new_with_name(i)).unwrap();
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
            tree.put(&i, &TestValue::new_with_name(i)).unwrap();
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
            tree.put(&i, &TestValue::new_with_name(i)).unwrap();
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
            tree.put(&key, &TestValue::new_with_name(key)).unwrap();
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

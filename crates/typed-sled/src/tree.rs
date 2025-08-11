use std::{
    marker::PhantomData,
    ops::{Bound, RangeBounds},
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
#[derive(Debug, Clone)]
pub struct SledTree<S: Schema> {
    pub(crate) inner: Tree,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledTree<S> {
    /// Creates a new typed tree wrapper.
    pub fn new(inner: Tree) -> Self {
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

    /// Returns `true` if the `SledTree` contains a value for the specified key
    pub fn contains_key(&self, key: &S::Key) -> Result<bool> {
        let key = key.encode_key()?;
        Ok(self.inner.contains_key(key)?)
    }

    /// Returns the first key-value pair in the tree.
    pub fn first(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.first()?.map(decode_pair::<S>).transpose()
    }

    /// Returns the last key-value pair in the tree.
    pub fn last(&self) -> Result<Option<(S::Key, S::Value)>> {
        self.inner.last()?.map(decode_pair::<S>).transpose()
    }

    /// Compares and swaps only if the value equals the old value.
    pub fn compare_and_swap(
        &self,
        key: S::Key,
        old: Option<S::Value>,
        new: Option<S::Value>,
    ) -> Result<()> {
        let key = key.encode_key()?;
        let old = old.as_ref().map(S::Value::encode_value).transpose()?;
        let new = new.as_ref().map(S::Value::encode_value).transpose()?;
        self.inner.compare_and_swap(key, old, new)??;
        Ok(())
    }

    /// Applies a batch of operations atomically.
    pub fn apply_batch(&self, batch: SledBatch<S>) -> Result<()> {
        self.inner.apply_batch(batch.inner)?;
        let _ = self.inner.flush();
        Ok(())
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

impl<S: Schema> std::fmt::Debug for SledTransactionalTree<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SledTransactionalTree")
            .field("tree_name", &S::TREE_NAME.0)
            .field("schema", &std::any::type_name::<S>())
            .finish()
    }
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

    /// Returns `true` if the `SledTree` contains a value for the specified key
    pub fn contains_key(&self, key: &S::Key) -> Result<bool> {
        let key = key.encode_key()?;
        Ok(self.inner.get(key)?.is_some())
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

impl<S: Schema> std::fmt::Debug for SledTreeIter<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SledTreeIter")
            .field("tree_name", &S::TREE_NAME.0)
            .field("schema", &std::any::type_name::<S>())
            .finish()
    }
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
    use super::*;
    use crate::test_utils::*;

    fn create_test_tree() -> Result<SledTree<TestSchema1>> {
        create_temp_tree::<TestSchema1>()
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
        tree.insert(&1, &TestValue::alice()).unwrap();
        tree.insert(&3, &TestValue::charlie()).unwrap();
        tree.insert(&2, &TestValue::bob()).unwrap();

        let items: Result<Vec<_>> = tree.iter().collect();
        let items = items.unwrap();

        // Should be sorted by key
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 1);
        assert_eq!(items[1].0, 2);
        assert_eq!(items[2].0, 3);
        assert_test_values_eq(&items[0].1, &TestValue::alice());
        assert_test_values_eq(&items[1].1, &TestValue::bob());
        assert_test_values_eq(&items[2].1, &TestValue::charlie());
    }

    #[test]
    fn test_iter_backward() {
        let tree = create_test_tree().unwrap();

        // Insert test data
        tree.insert(&1, &TestValue::alice()).unwrap();
        tree.insert(&3, &TestValue::charlie()).unwrap();
        tree.insert(&2, &TestValue::bob()).unwrap();

        let items: Result<Vec<_>> = tree.iter().rev().collect();
        let items = items.unwrap();
        println!("{items:?}");

        // Should be reverse sorted by key
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, 3);
        assert_eq!(items[1].0, 2);
        assert_eq!(items[2].0, 1);
        assert_test_values_eq(&items[0].1, &TestValue::charlie());
        assert_test_values_eq(&items[1].1, &TestValue::bob());
        assert_test_values_eq(&items[2].1, &TestValue::alice());
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

    #[test]
    fn test_basic_insert_get_operations() {
        let tree = create_test_tree().unwrap();

        // Test insert and get
        tree.insert(&1, &TestValue::alice()).unwrap();
        let value = tree.get(&1).unwrap();
        assert!(value.is_some());
        assert_test_values_eq(&TestValue::alice(), &value.unwrap());

        // Test get non-existent key
        let non_existent = tree.get(&999).unwrap();
        assert!(non_existent.is_none());
    }

    #[test]
    fn test_remove_operations() {
        let tree = create_test_tree().unwrap();

        // Insert and verify
        tree.insert(&1, &TestValue::alice()).unwrap();
        assert!(tree.contains_key(&1).unwrap());

        // Remove and verify
        tree.remove(&1).unwrap();
        assert!(!tree.contains_key(&1).unwrap());
        assert!(tree.get(&1).unwrap().is_none());

        // Remove non-existent key should not error
        tree.remove(&999).unwrap();
    }

    #[test]
    fn test_is_empty() {
        let tree = create_test_tree().unwrap();

        // New tree should be empty
        assert!(tree.is_empty());

        // After insert, should not be empty
        tree.insert(&1, &TestValue::alice()).unwrap();
        assert!(!tree.is_empty());

        // After remove, should be empty again
        tree.remove(&1).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn test_contains_key() {
        let tree = create_test_tree().unwrap();

        // Key should not exist initially
        assert!(!tree.contains_key(&1).unwrap());

        // Insert and check
        tree.insert(&1, &TestValue::alice()).unwrap();
        assert!(tree.contains_key(&1).unwrap());

        // Remove and check
        tree.remove(&1).unwrap();
        assert!(!tree.contains_key(&1).unwrap());
    }

    #[test]
    fn test_first_and_last() {
        let tree = create_test_tree().unwrap();

        // Empty tree should return None
        assert!(tree.first().unwrap().is_none());
        assert!(tree.last().unwrap().is_none());

        // Insert data out of order
        tree.insert(&3, &TestValue::charlie()).unwrap();
        tree.insert(&1, &TestValue::alice()).unwrap();
        tree.insert(&2, &TestValue::bob()).unwrap();

        // First should be key 1
        let first = tree.first().unwrap().unwrap();
        assert_eq!(first.0, 1);
        assert_test_values_eq(&first.1, &TestValue::alice());

        // Last should be key 3
        let last = tree.last().unwrap().unwrap();
        assert_eq!(last.0, 3);
        assert_test_values_eq(&last.1, &TestValue::charlie());
    }

    #[test]
    fn test_compare_and_swap() {
        let tree = create_test_tree().unwrap();

        // CAS on non-existent key with None expected
        tree.compare_and_swap(1, None, Some(TestValue::alice()))
            .unwrap();
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&value, &TestValue::alice());

        // CAS with correct old value
        tree.compare_and_swap(1, Some(TestValue::alice()), Some(TestValue::bob()))
            .unwrap();
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&value, &TestValue::bob());

        // CAS to remove (set to None)
        tree.compare_and_swap(1, Some(TestValue::bob()), None)
            .unwrap();
        assert!(!tree.contains_key(&1).unwrap());
    }

    #[test]
    fn test_overwrite_existing_key() {
        let tree = create_test_tree().unwrap();

        // Insert initial value
        tree.insert(&1, &TestValue::alice()).unwrap();
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&value, &TestValue::alice());

        // Overwrite with new value
        tree.insert(&1, &TestValue::bob()).unwrap();
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&value, &TestValue::bob());
    }

    #[test]
    fn test_multiple_concurrent_operations() {
        let tree = create_test_tree().unwrap();

        // Insert multiple keys
        for i in 1..=10 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        // Verify all keys exist
        for i in 1..=10 {
            assert!(tree.contains_key(&i).unwrap());
            let value = tree.get(&i).unwrap().unwrap();
            assert_test_values_eq(&value, &TestValue::new_with_name(i));
        }

        // Remove even keys
        for i in (2..=10).step_by(2) {
            tree.remove(&i).unwrap();
        }

        // Verify odd keys remain, even keys are gone
        for i in 1..=10 {
            if i % 2 == 1 {
                assert!(tree.contains_key(&i).unwrap());
            } else {
                assert!(!tree.contains_key(&i).unwrap());
            }
        }
    }

    #[test]
    fn test_range_edge_cases() {
        let tree = create_test_tree().unwrap();

        // Insert data
        for i in 1..=10 {
            tree.insert(&i, &TestValue::new_with_name(i)).unwrap();
        }

        // Test empty range
        let items: Result<Vec<_>> = tree.range(5..5).unwrap().collect();
        assert!(items.unwrap().is_empty());

        // Test range beyond data
        let items: Result<Vec<_>> = tree.range(15..20).unwrap().collect();
        assert!(items.unwrap().is_empty());

        // Test single item range
        let items: Result<Vec<_>> = tree.range(5..=5).unwrap().collect();
        let items = items.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 5);
    }
}

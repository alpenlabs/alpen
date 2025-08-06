use std::marker::PhantomData;

use sled::Batch;

use crate::{KeyCodec, Schema, ValueCodec, error::Result};

/// Type-safe wrapper around a sled batch for atomic operations.
#[derive(Debug)]
pub struct SledBatch<S: Schema> {
    pub(crate) inner: Batch,
    _phantom: PhantomData<S>,
}

impl<S: Schema> SledBatch<S> {
    /// Creates a new empty batch.
    pub fn new() -> Self {
        Self {
            inner: Batch::default(),
            _phantom: PhantomData,
        }
    }

    /// Adds an insert operation to the batch.
    pub fn insert(&mut self, key: S::Key, value: S::Value) -> Result<()> {
        let key = key.encode_key()?;
        let value = value.encode_value()?;
        self.inner.insert(key, value);
        Ok(())
    }

    /// Adds a remove operation to the batch.
    pub fn remove(&mut self, key: S::Key) -> Result<()> {
        let key = key.encode_key()?;
        self.inner.remove(key);
        Ok(())
    }
}

impl<S: Schema> Default for SledBatch<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    #[test]
    fn test_sled_batch_new() {
        let _batch = SledBatch::<TestSchema1>::new();

        // Should create successfully without panicking
        // (No way to directly test if batch is empty in sled)
    }

    #[test]
    fn test_sled_batch_default() {
        let _batch = SledBatch::<TestSchema1>::default();

        // Default should create successfully
        // (No way to directly test if batch is empty in sled)
    }

    #[test]
    fn test_batch_insert_single() {
        let mut batch = SledBatch::<TestSchema1>::new();

        // Insert should succeed
        let result = batch.insert(1, TestValue::alice());
        assert!(result.is_ok());

        // Insert completed successfully
    }

    #[test]
    fn test_batch_insert_multiple() {
        let mut batch = SledBatch::<TestSchema1>::new();

        // Insert multiple values
        batch.insert(1, TestValue::alice()).unwrap();
        batch.insert(2, TestValue::bob()).unwrap();
        batch.insert(3, TestValue::charlie()).unwrap();

        // Multiple inserts completed successfully
    }

    #[test]
    fn test_batch_remove_single() {
        let mut batch = SledBatch::<TestSchema1>::new();

        // Remove should succeed even if key doesn't exist
        let result = batch.remove(1);
        assert!(result.is_ok());

        // Remove completed successfully
    }

    #[test]
    fn test_batch_mixed_operations() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Pre-populate with some data
        tree.insert(&2, &TestValue::bob()).unwrap();
        tree.insert(&5, &TestValue::new(5, "Existing")).unwrap();

        let mut batch = SledBatch::<TestSchema1>::new();

        // Mix of operations:
        batch.insert(1, TestValue::alice()).unwrap(); // Insert new key
        batch.remove(2).unwrap(); // Remove existing key
        batch.insert(3, TestValue::charlie()).unwrap(); // Insert new key
        batch.remove(1).unwrap(); // Remove key we just added in batch
        batch.insert(4, TestValue::new(4, "David")).unwrap(); // Insert new key
        batch.remove(99).unwrap(); // Remove non-existent key (should be no-op)

        // Apply batch and verify specific results
        tree.apply_batch(batch).unwrap();

        // Key 1: inserted then removed in batch - should not exist
        assert!(!tree.contains_key(&1).unwrap());

        // Key 2: existed, then removed in batch - should not exist
        assert!(!tree.contains_key(&2).unwrap());

        // Key 3: inserted in batch - should exist
        assert!(tree.contains_key(&3).unwrap());
        let val3 = tree.get(&3).unwrap().unwrap();
        assert_test_values_eq(&TestValue::charlie(), &val3);

        // Key 4: inserted in batch - should exist
        assert!(tree.contains_key(&4).unwrap());
        let val4 = tree.get(&4).unwrap().unwrap();
        assert_test_values_eq(&TestValue::new(4, "David"), &val4);

        // Key 5: existed before, not touched in batch - should still exist
        assert!(tree.contains_key(&5).unwrap());
        let val5 = tree.get(&5).unwrap().unwrap();
        assert_test_values_eq(&TestValue::new(5, "Existing"), &val5);

        // Key 99: removed but didn't exist - should still not exist
        assert!(!tree.contains_key(&99).unwrap());
    }

    #[test]
    fn test_batch_apply_to_tree() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();
        let mut batch = SledBatch::<TestSchema1>::new();

        // Add operations to batch
        batch.insert(1, TestValue::alice()).unwrap();
        batch.insert(2, TestValue::bob()).unwrap();
        batch.insert(3, TestValue::charlie()).unwrap();

        // Apply batch to tree
        tree.apply_batch(batch).unwrap();

        // Verify all data was inserted
        let value1 = tree.get(&1).unwrap().unwrap();
        let value2 = tree.get(&2).unwrap().unwrap();
        let value3 = tree.get(&3).unwrap().unwrap();

        assert_test_values_eq(&TestValue::alice(), &value1);
        assert_test_values_eq(&TestValue::bob(), &value2);
        assert_test_values_eq(&TestValue::charlie(), &value3);
    }

    #[test]
    fn test_batch_remove_existing_key() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // First insert some data directly
        tree.insert(&1, &TestValue::alice()).unwrap();
        tree.insert(&2, &TestValue::bob()).unwrap();
        assert!(tree.contains_key(&1).unwrap());
        assert!(tree.contains_key(&2).unwrap());

        // Create batch to remove one key
        let mut batch = SledBatch::<TestSchema1>::new();
        batch.remove(1).unwrap();

        // Apply batch
        tree.apply_batch(batch).unwrap();

        // Key 1 should be removed, key 2 should remain
        assert!(!tree.contains_key(&1).unwrap());
        assert!(tree.contains_key(&2).unwrap());
    }

    #[test]
    fn test_batch_atomicity() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Insert initial data
        tree.insert(&1, &TestValue::alice()).unwrap();

        // Create batch with multiple operations
        let mut batch = SledBatch::<TestSchema1>::new();
        batch.insert(2, TestValue::bob()).unwrap();
        batch.insert(3, TestValue::charlie()).unwrap();
        batch.remove(1).unwrap(); // Remove existing key
        batch.insert(4, TestValue::new(4, "David")).unwrap();

        // Apply batch atomically
        tree.apply_batch(batch).unwrap();

        // All operations should have been applied together
        assert!(!tree.contains_key(&1).unwrap()); // Removed
        assert!(tree.contains_key(&2).unwrap()); // Inserted
        assert!(tree.contains_key(&3).unwrap()); // Inserted
        assert!(tree.contains_key(&4).unwrap()); // Inserted

        // Verify inserted values
        let value2 = tree.get(&2).unwrap().unwrap();
        let value3 = tree.get(&3).unwrap().unwrap();
        let value4 = tree.get(&4).unwrap().unwrap();

        assert_test_values_eq(&TestValue::bob(), &value2);
        assert_test_values_eq(&TestValue::charlie(), &value3);
        assert_test_values_eq(&TestValue::new(4, "David"), &value4);
    }

    #[test]
    fn test_batch_overwrite_operations() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Insert initial value
        tree.insert(&1, &TestValue::alice()).unwrap();

        // Create batch that overwrites the same key multiple times
        let mut batch = SledBatch::<TestSchema1>::new();
        batch.insert(1, TestValue::bob()).unwrap(); // Overwrite with Bob
        batch.insert(1, TestValue::charlie()).unwrap(); // Overwrite with Charlie

        // Apply batch
        tree.apply_batch(batch).unwrap();

        // Final value should be Charlie (last write wins)
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::charlie(), &value);
    }

    #[test]
    fn test_batch_insert_then_remove_same_key() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Create batch that inserts then removes the same key
        let mut batch = SledBatch::<TestSchema1>::new();
        batch.insert(1, TestValue::alice()).unwrap();
        batch.remove(1).unwrap();

        // Apply batch
        tree.apply_batch(batch).unwrap();

        // Key should not exist (remove should win)
        assert!(!tree.contains_key(&1).unwrap());
    }

    #[test]
    fn test_batch_empty_application() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Insert some initial data
        tree.insert(&1, &TestValue::alice()).unwrap();

        // Apply empty batch
        let batch = SledBatch::<TestSchema1>::new();
        tree.apply_batch(batch).unwrap();

        // Original data should remain unchanged
        assert!(tree.contains_key(&1).unwrap());
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::alice(), &value);
    }

    #[test]
    fn test_batch_large_operations() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();
        let mut batch = SledBatch::<TestSchema1>::new();

        // Add many operations to batch
        for i in 0..1000 {
            batch.insert(i, TestValue::new_with_name(i)).unwrap();
        }

        // Apply large batch
        tree.apply_batch(batch).unwrap();

        // Verify all data was inserted
        for i in 0..1000 {
            let value = tree.get(&i).unwrap();
            assert!(value.is_some());
            assert_test_values_eq(&TestValue::new_with_name(i), &value.unwrap());
        }
    }

    #[test]
    fn test_batch_multiple_schema_types() {
        // Test that batches are properly typed to their schema
        let tree1 = create_temp_tree::<TestSchema1>().unwrap();
        let tree2 = create_temp_tree::<TestSchema2>().unwrap();

        let mut batch1 = SledBatch::<TestSchema1>::new();
        let mut batch2 = SledBatch::<TestSchema2>::new();

        // Add data to different batches
        batch1.insert(1, TestValue::alice()).unwrap();
        batch2.insert(1, TestValue::bob()).unwrap();

        // Apply batches to respective trees
        tree1.apply_batch(batch1).unwrap();
        tree2.apply_batch(batch2).unwrap();

        // Each tree should have its own data
        let value1 = tree1.get(&1).unwrap().unwrap();
        let value2 = tree2.get(&1).unwrap().unwrap();

        assert_test_values_eq(&TestValue::alice(), &value1);
        assert_test_values_eq(&TestValue::bob(), &value2);

        // Verify that trees don't interfere with each other
        assert!(!tree1.contains_key(&2).unwrap());
        assert!(!tree2.contains_key(&2).unwrap());
    }

    #[test]
    fn test_batch_operation_order() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();
        let mut batch = SledBatch::<TestSchema1>::new();

        // Test that operations in batch maintain specific order behavior
        batch.insert(1, TestValue::alice()).unwrap(); // Insert Alice
        batch.insert(1, TestValue::bob()).unwrap(); // Overwrite with Bob
        batch.insert(1, TestValue::charlie()).unwrap(); // Overwrite with Charlie

        tree.apply_batch(batch).unwrap();

        // Final value should be Charlie (last write in batch wins)
        let final_value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::charlie(), &final_value);
    }

    #[test]
    fn test_batch_remove_then_insert_same_key() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Pre-populate tree
        tree.insert(&1, &TestValue::alice()).unwrap();

        let mut batch = SledBatch::<TestSchema1>::new();

        // Remove then insert same key in batch
        batch.remove(1).unwrap();
        batch.insert(1, TestValue::bob()).unwrap();

        tree.apply_batch(batch).unwrap();

        // Key should exist with new value (insert after remove)
        assert!(tree.contains_key(&1).unwrap());
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::bob(), &value);
    }

    #[test]
    fn test_batch_concurrent_key_modifications() {
        let tree = create_temp_tree::<TestSchema1>().unwrap();

        // Test multiple modifications to different aspects of the same key
        tree.insert(&1, &TestValue::new(100, "Original")).unwrap();

        let mut batch = SledBatch::<TestSchema1>::new();

        // Multiple operations on the same key with different values
        batch.insert(1, TestValue::new(1, "First")).unwrap();
        batch.insert(1, TestValue::new(2, "Second")).unwrap();
        batch.remove(1).unwrap();
        batch.insert(1, TestValue::new(3, "Third")).unwrap();

        tree.apply_batch(batch).unwrap();

        // Final state should reflect last insert after remove
        let value = tree.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::new(3, "Third"), &value);
        assert_eq!(value.id, 3);
        assert_eq!(value.name, "Third");
    }
}

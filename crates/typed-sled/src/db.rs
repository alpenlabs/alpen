use std::sync::Arc;

use dashmap::DashMap;
use sled::{Db, Tree};

use crate::{
    error::Result,
    schema::{Schema, TreeName},
    tree::SledTree,
};

/// A type-safe wrapper around sled database with schema-based tree management.
#[derive(Debug)]
pub struct SledDb {
    /// Mapping of treenames to sled tree.
    inner_trees: DashMap<TreeName, Arc<Tree>>,
    /// The actual sled db.
    inner_db: Arc<Db>,
}

impl SledDb {
    /// Creates a new typed sled database wrapper.
    pub fn new(inner_db: Arc<Db>) -> Result<Self> {
        Ok(Self {
            inner_db,
            inner_trees: DashMap::new(),
        })
    }

    /// Gets or creates a typed tree for the given schema.
    pub fn get_tree<S: Schema>(&self) -> Result<SledTree<S>> {
        if let Some(tree) = self.inner_trees.get(&S::TREE_NAME) {
            return Ok(SledTree::new(tree.clone()));
        }

        // Create the tree
        let tree_name = S::TREE_NAME.into_inner();
        let tree = Arc::new(self.inner_db.open_tree(tree_name)?);

        let entry = self.inner_trees.entry(S::TREE_NAME);
        let final_tree = entry.or_insert(tree);
        Ok(SledTree::new(final_tree.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn test_sled_db_new() {
        let sled_db = create_temp_sled_db();
        let db = SledDb::new(sled_db);

        assert!(db.is_ok());
        let db = db.unwrap();

        // Verify internal structure
        assert_eq!(db.inner_trees.len(), 0);
    }

    #[test]
    fn test_get_tree_creates_new_tree() {
        let db = create_test_db().unwrap();

        // Get a tree for the first time
        let tree1 = db.get_tree::<TestSchema1>();
        assert!(tree1.is_ok());

        // Verify tree was cached
        assert_eq!(db.inner_trees.len(), 1);
        assert!(db.inner_trees.contains_key(&TestSchema1::TREE_NAME));
    }

    #[test]
    fn test_get_tree_returns_cached_tree() {
        let db = create_test_db().unwrap();

        // Get tree twice
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        let tree2 = db.get_tree::<TestSchema1>().unwrap();

        // Should only have one cached entry
        assert_eq!(db.inner_trees.len(), 1);

        // Both trees should reference the same underlying Arc<Tree>
        assert_eq!(Arc::as_ptr(&tree1.inner), Arc::as_ptr(&tree2.inner));
    }

    #[test]
    fn test_get_multiple_different_trees() {
        let db = create_test_db().unwrap();

        // Get trees for different schemas
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        let tree2 = db.get_tree::<TestSchema2>().unwrap();
        let tree3 = db.get_tree::<TestSchema3>().unwrap();

        // Should have three cached entries
        assert_eq!(db.inner_trees.len(), 3);

        // Verify all trees are cached
        assert!(db.inner_trees.contains_key(&TestSchema1::TREE_NAME));
        assert!(db.inner_trees.contains_key(&TestSchema2::TREE_NAME));
        assert!(db.inner_trees.contains_key(&TestSchema3::TREE_NAME));

        // Trees should have different underlying Arc<Tree> instances
        assert_ne!(Arc::as_ptr(&tree1.inner), Arc::as_ptr(&tree2.inner));
        assert_ne!(Arc::as_ptr(&tree2.inner), Arc::as_ptr(&tree3.inner));
    }

    #[test]
    fn test_tree_operations_work_through_db() {
        let db = create_test_db().unwrap();
        let tree = db.get_tree::<TestSchema1>().unwrap();

        // Basic tree operations should work
        tree.insert(&1, &TestValue::alice()).unwrap();
        let retrieved = tree.get(&1).unwrap();

        assert!(retrieved.is_some());
        assert_test_values_eq(&TestValue::alice(), &retrieved.unwrap());
    }

    #[test]
    fn test_concurrent_tree_access() {
        let db = Arc::new(create_test_db().unwrap());
        let mut handles = vec![];

        // Spawn multiple threads that all try to get the same tree
        for i in 0..10 {
            let db_clone = Arc::clone(&db);
            let handle = thread::spawn(move || {
                let tree = db_clone.get_tree::<TestSchema1>().unwrap();

                // Each thread inserts its own data
                let value = TestValue::new(i, &format!("thread_{}", i));
                tree.insert(&i, &value).unwrap();

                // Verify insertion worked
                let retrieved = tree.get(&i).unwrap();
                assert!(retrieved.is_some());
                assert_test_values_eq(&value, &retrieved.unwrap());

                tree
            });
            handles.push(handle);
        }

        // Wait for all threads to complete and collect trees
        let trees: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Verify only one tree was cached (all threads got the same tree)
        assert_eq!(db.inner_trees.len(), 1);

        // All trees should reference the same underlying Arc<Tree>
        let first_tree_ptr = Arc::as_ptr(&trees[0].inner);
        for tree in &trees[1..] {
            assert_eq!(Arc::as_ptr(&tree.inner), first_tree_ptr);
        }

        // Verify all data from different threads is accessible
        for i in 0..10 {
            let value = trees[0].get(&i).unwrap();
            assert!(value.is_some());
            let expected = TestValue::new(i, &format!("thread_{}", i));
            assert_test_values_eq(&expected, &value.unwrap());
        }
    }

    #[test]
    fn test_concurrent_different_tree_access() {
        let db = Arc::new(create_test_db().unwrap());
        let mut handles = vec![];

        // Spawn threads that access different trees
        for i in 0..3 {
            let db_clone = Arc::clone(&db);
            let handle = thread::spawn(move || match i {
                0 => {
                    let tree = db_clone.get_tree::<TestSchema1>().unwrap();
                    tree.insert(&1, &TestValue::alice()).unwrap();
                }
                1 => {
                    let tree = db_clone.get_tree::<TestSchema2>().unwrap();
                    tree.insert(&2, &TestValue::bob()).unwrap();
                }
                _ => {
                    let tree = db_clone.get_tree::<TestSchema3>().unwrap();
                    tree.insert(&3, &TestValue::charlie()).unwrap();
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Should have cached all three different trees
        assert_eq!(db.inner_trees.len(), 3);

        // Verify data isolation by getting fresh tree instances
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        let tree2 = db.get_tree::<TestSchema2>().unwrap();
        let tree3 = db.get_tree::<TestSchema3>().unwrap();

        // Each tree should have its own data
        assert!(tree1.get(&1).unwrap().is_some()); // Schema1 has key 1
        assert!(tree1.get(&2).unwrap().is_none()); // but not key 2
        assert!(tree1.get(&3).unwrap().is_none()); // or key 3

        assert!(tree2.get(&2).unwrap().is_some()); // Schema2 has key 2
        assert!(tree2.get(&1).unwrap().is_none()); // but not key 1
        assert!(tree2.get(&3).unwrap().is_none()); // or key 3

        assert!(tree3.get(&3).unwrap().is_some()); // Schema3 has key 3
        assert!(tree3.get(&1).unwrap().is_none()); // but not key 1
        assert!(tree3.get(&2).unwrap().is_none()); // or key 2
    }

    #[test]
    fn test_tree_cache_consistency_after_operations() {
        let db = create_test_db().unwrap();

        // Get tree and perform operations
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        tree1.insert(&1, &TestValue::alice()).unwrap();

        // Get same tree again
        let tree2 = db.get_tree::<TestSchema1>().unwrap();

        // Should be able to read data inserted with first tree instance
        let value = tree2.get(&1).unwrap();
        assert!(value.is_some());
        assert_test_values_eq(&TestValue::alice(), &value.unwrap());

        // Cache should still have only one entry
        assert_eq!(db.inner_trees.len(), 1);
    }
}

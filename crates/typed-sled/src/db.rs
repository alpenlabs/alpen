use std::sync::Arc;

use dashmap::DashMap;
use sled::{Db, Tree};

use crate::{
    error::Result,
    schema::{Schema, TreeName},
    tree::SledTree,
};

pub struct SledDb {
    /// Mapping of treenames to sled tree.
    inner_trees: DashMap<TreeName, Arc<Tree>>,
    /// The actual sled db.
    inner_db: Arc<Db>,
}

impl SledDb {
    pub fn new(inner_db: Arc<Db>) -> Result<Self> {
        Ok(Self {
            inner_db,
            inner_trees: DashMap::new(),
        })
    }

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

    // TODO: Implement proper transaction methods
    // /// Transactional function for two Schemas.
    // pub fn transaction<S1: Schema, S2: Schema>(
    //     &self,
    //     txfn: impl Fn(&(TransactionalTree, TransactionalTree)) -> DbResult<()>,
    // ) -> DbResult<()> {
    //     // Implementation needed for typed transactional operations
    //     todo!("Implement typed transactions")
    // }
}

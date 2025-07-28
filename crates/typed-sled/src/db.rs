use std::sync::Arc;

use dashmap::DashMap;
use sled::{Db, Transactional, Tree, transaction::ConflictableTransactionError};

use crate::{
    error::{Error, Result},
    schema::{Schema, TreeName},
    tree::{SledTransactionalTree, SledTree},
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

    pub fn transaction<F, S1: Schema, S2: Schema>(&self, func: F) -> Result<()>
    where
        F: Fn((&SledTransactionalTree<S1>, &SledTransactionalTree<S2>)) -> Result<()>,
    {
        let t1: SledTree<S1> = self.get_tree()?;
        let t2: SledTree<S2> = self.get_tree()?;
        (&*t1.inner, &*t2.inner)
            .transaction(|(t1, t2)| {
                let st1 = SledTransactionalTree::<S1>::new(t1.clone());
                let st2 = SledTransactionalTree::<S2>::new(t2.clone());
                func((&st1, &st2)).map_err(ConflictableTransactionError::Abort)
            })
            .map_err(|e| Error::TransactionError(e))
    }
}

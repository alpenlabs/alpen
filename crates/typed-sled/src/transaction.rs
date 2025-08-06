use sled::{
    Transactional,
    transaction::{ConflictableTransactionResult, TransactionResult},
};

use crate::{Schema, SledTree, tree::SledTransactionalTree};

/// Trait for performing transactions on typed sled trees.
pub trait SledTransactional {
    type View;

    /// Executes a function within a transaction context.
    fn transaction<F, R, E>(&self, func: F) -> TransactionResult<R, E>
    where
        F: Fn(Self::View) -> ConflictableTransactionResult<R, E>;
}

/* Definition of implementations like this for various tuple arities
 *
impl<S1: Schema> SledTransactional for (&SledTree<S1>,) {
    type View = (SledTransactionalTree<S1>,);

    fn transaction<F, R, E>(&self, func: F) -> TransactionResult<R, E>
    where
        F: Fn(Self::View) -> ConflictableTransactionResult<R, E>,
    {
        (&*self.0.inner,).transaction(|(t,)| {
            let st = SledTransactionalTree::<S1>::new(t.clone());
            func((st,))
        })
    }
}
*/

/// Implements [`SledTransactional`] trait for various [`SledTree`] tuples. This provides a
/// similar interface to what [sled provides]
/// (https://docs.rs/sled/latest/sled/struct.Tree.html#method.transaction).
macro_rules! impl_sled_transactional {
    ($(($idx:tt, $schema:ident, $var:ident)),+) => {
        /// Impl for owned `SledTree`
        impl<$($schema: Schema),+> SledTransactional for ($(SledTree<$schema>),+,) {
            type View = ($(SledTransactionalTree<$schema>),+,);

            fn transaction<F, R, E>(&self, func: F) -> TransactionResult<R, E>
            where
                F: Fn(Self::View) -> ConflictableTransactionResult<R, E>,
            {
                ($(&*self.$idx.inner),+,).transaction(|($($var),+,)| {
                    func(($(SledTransactionalTree::<$schema>::new($var.clone())),+,))
                })
            }
        }

        // Impl for `SledTree` reference
        impl<$($schema: Schema),+> SledTransactional for ($(&SledTree<$schema>),+,) {
            type View = ($(SledTransactionalTree<$schema>),+,);

            fn transaction<F, R, E>(&self, func: F) -> TransactionResult<R, E>
            where
                F: Fn(Self::View) -> ConflictableTransactionResult<R, E>,
            {
                ($(&*self.$idx.inner),+,).transaction(|($($var),+,)| {
                    func(($(SledTransactionalTree::<$schema>::new($var.clone())),+,))
                })
            }
        }
    };
}

impl_sled_transactional!((0, S0, t0));
impl_sled_transactional!((0, S0, t0), (1, S1, t1));
impl_sled_transactional!((0, S0, t0), (1, S1, t1), (2, S2, t2));
impl_sled_transactional!((0, S0, t0), (1, S1, t1), (2, S2, t2), (3, S3, t3));
impl_sled_transactional!(
    (0, S0, t0),
    (1, S1, t1),
    (2, S2, t2),
    (3, S3, t3),
    (4, S4, t4)
);
impl_sled_transactional!(
    (0, S0, t0),
    (1, S1, t1),
    (2, S2, t2),
    (3, S3, t3),
    (4, S4, t4),
    (5, S5, t5)
);

#[cfg(test)]
mod tests {
    use sled::transaction::TransactionResult;

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn test_single_tree_transaction_insert_and_get() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction(|(tx_tree1,)| {
                let value = TestValue::alice();
                tx_tree1.insert(&1, &value)?;

                let retrieved = tx_tree1.get(&1)?.unwrap();
                assert_eq!(retrieved, value);
                Ok(())
            });

        assert!(result.is_ok());

        // Verify data persisted after transaction
        let retrieved = tree1.get(&1).unwrap().unwrap();
        assert_test_values_eq(&TestValue::alice(), &retrieved);
    }

    #[test]
    fn test_single_tree_transaction_remove() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        // Insert initial data
        tree1.insert(&1, &TestValue::alice()).unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction(|(tx_tree1,)| {
                assert!(tx_tree1.contains_key(&1)?);
                tx_tree1.remove(&1)?;
                assert!(!tx_tree1.contains_key(&1)?);
                Ok(())
            });

        assert!(result.is_ok());

        // Verify data removed after transaction
        assert!(!tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_multi_tree_transaction_two_trees() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        let tree2 = db.get_tree::<TestSchema2>().unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (&tree1, &tree2).transaction(|(tx_tree1, tx_tree2)| {
                tx_tree1.insert(&1, &TestValue::alice())?;
                tx_tree2.insert(&2, &TestValue::bob())?;

                // Verify both are accessible within transaction
                assert!(tx_tree1.contains_key(&1)?);
                assert!(tx_tree2.contains_key(&2)?);

                Ok(())
            });

        assert!(result.is_ok());

        // Verify data persisted in both trees after transaction
        assert!(tree1.contains_key(&1).unwrap());
        assert!(tree2.contains_key(&2).unwrap());
    }

    #[test]
    fn test_transaction_rollback_on_error() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let result: TransactionResult<(), &'static str> = (&tree1,).transaction(|(tx_tree1,)| {
            let _ = tx_tree1.insert(&1, &TestValue::alice());

            // Simulate an error that should cause rollback
            Err(sled::transaction::ConflictableTransactionError::Abort(
                "intentional error",
            ))
        });

        // Transaction should fail
        assert!(result.is_err());

        // Data should not be persisted due to rollback
        assert!(!tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_transactional_tree_contains_key() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction(|(tx_tree1,)| {
                assert!(!tx_tree1.contains_key(&1)?);
                tx_tree1.insert(&1, &TestValue::alice())?;
                assert!(tx_tree1.contains_key(&1)?);
                Ok(())
            });

        assert!(result.is_ok());
    }

    #[test]
    fn test_transactional_tree_get_nonexistent() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction(|(tx_tree1,)| {
                let value = tx_tree1.get(&999)?;
                assert!(value.is_none());
                Ok(())
            });

        assert!(result.is_ok());
    }

    #[test]
    fn test_owned_tree_transaction() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let result: TransactionResult<(), crate::error::Error> =
            (tree1.clone(),).transaction(|(tx_tree1,)| {
                let value = TestValue::alice();
                tx_tree1.insert(&1, &value)?;
                Ok(())
            });

        assert!(result.is_ok());
        assert!(tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_three_tree_transaction() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();
        let tree2 = db.get_tree::<TestSchema2>().unwrap();
        let tree3 = db.get_tree::<TestSchema3>().unwrap(); // Same schema as tree1

        let result: TransactionResult<(), crate::error::Error> = (&tree1, &tree2, &tree3)
            .transaction(|(tx_tree1, tx_tree2, tx_tree3)| {
                let value1 = TestValue::alice();
                let value2 = TestValue::bob();
                tx_tree1.insert(&1, &value1)?;
                tx_tree2.insert(&2, &value2)?;
                tx_tree3.insert(&3, &TestValue::charlie())?;

                // All operations should succeed within the transaction
                assert!(tx_tree1.contains_key(&1)?);
                assert!(tx_tree2.contains_key(&2)?);
                assert!(tx_tree3.contains_key(&3)?);

                Ok(())
            });

        assert!(result.is_ok());
    }
}

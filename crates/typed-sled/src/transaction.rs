use std::time::Duration;

use sled::{
    Transactional,
    transaction::{ConflictableTransactionResult, TransactionError, TransactionResult},
};

use crate::{Schema, SledTree, tree::SledTransactionalTree};

/// Backoff policy trait for retry logic.
pub trait Backoff {
    /// Base delay in ms.
    fn base_delay_ms(&self) -> u64;

    /// Generates next delay given current delay.
    fn next_delay_ms(&self, curr_delay_ms: u64) -> u64;
}

/// Exponential backoff strategy.
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    pub base_delay_ms: u64,
    pub multiplier: f64,
    pub max_delay_ms: u64,
}

impl ExponentialBackoff {
    pub fn new(base_delay_ms: u64, multiplier: f64, max_delay_ms: u64) -> Self {
        Self {
            base_delay_ms,
            multiplier,
            max_delay_ms,
        }
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base_delay_ms: 10,
            multiplier: 2.0,
            max_delay_ms: 5000,
        }
    }
}

impl Backoff for ExponentialBackoff {
    fn base_delay_ms(&self) -> u64 {
        self.base_delay_ms
    }

    fn next_delay_ms(&self, curr_delay_ms: u64) -> u64 {
        let next = (curr_delay_ms as f64 * self.multiplier) as u64;
        std::cmp::min(next, self.max_delay_ms)
    }
}

/// Linear backoff strategy.
#[derive(Debug, Clone)]
pub struct LinearBackoff {
    pub base_delay_ms: u64,
    pub increment_ms: u64,
    pub max_delay_ms: u64,
}

impl LinearBackoff {
    pub fn new(base_delay_ms: u64, increment_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            base_delay_ms,
            increment_ms,
            max_delay_ms,
        }
    }
}

impl Default for LinearBackoff {
    fn default() -> Self {
        Self {
            base_delay_ms: 10,
            increment_ms: 10,
            max_delay_ms: 1000,
        }
    }
}

impl Backoff for LinearBackoff {
    fn base_delay_ms(&self) -> u64 {
        self.base_delay_ms
    }

    fn next_delay_ms(&self, curr_delay_ms: u64) -> u64 {
        std::cmp::min(curr_delay_ms + self.increment_ms, self.max_delay_ms)
    }
}

/// Constant backoff strategy.
#[derive(Debug, Clone)]
pub struct ConstantBackoff {
    pub delay_ms: u64,
}

impl ConstantBackoff {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms }
    }
}

impl Default for ConstantBackoff {
    fn default() -> Self {
        Self { delay_ms: 100 }
    }
}

impl Backoff for ConstantBackoff {
    fn base_delay_ms(&self) -> u64 {
        self.delay_ms
    }

    fn next_delay_ms(&self, _curr_delay_ms: u64) -> u64 {
        self.delay_ms
    }
}

/// Trait for performing transactions on typed sled trees.
pub trait SledTransactional {
    type View;

    /// Executes a function within a transaction context.
    fn transaction<F, R, E>(&self, func: F) -> TransactionResult<R, E>
    where
        F: Fn(Self::View) -> ConflictableTransactionResult<R, E>;

    /// Executes a function within a transaction context with retry and backoff.
    /// Only retries on storage conflicts, not on user aborts.
    fn transaction_with_retry<F, R, E, B>(
        &self,
        backoff: B,
        max_retries: usize,
        func: F,
    ) -> TransactionResult<R, E>
    where
        F: Fn(Self::View) -> ConflictableTransactionResult<R, E>,
        B: Backoff,
    {
        let mut attempts = 0;
        let mut delay_ms = backoff.base_delay_ms();

        loop {
            match self.transaction(&func) {
                Ok(result) => return Ok(result),
                Err(TransactionError::Abort(err)) => {
                    // User explicitly aborted, don't retry
                    return Err(TransactionError::Abort(err));
                }
                Err(TransactionError::Storage(storage_err)) => {
                    if attempts >= max_retries {
                        return Err(TransactionError::Storage(storage_err));
                    }

                    // Only retry on storage conflicts (like write conflicts)
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    delay_ms = backoff.next_delay_ms(delay_ms);
                    attempts += 1;
                }
            }
        }
    }
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
                ($(&self.$idx.inner),+,).transaction(|($($var),+,)| {
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
                ($(&self.$idx.inner),+,).transaction(|($($var),+,)| {
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

    #[test]
    fn test_retry_with_exponential_backoff() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let backoff = ExponentialBackoff::new(1, 2.0, 100);
        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction_with_retry(backoff, 3, |(tx_tree1,)| {
                let value = TestValue::alice();
                tx_tree1.insert(&1, &value)?;
                Ok(())
            });

        assert!(result.is_ok());
        assert!(tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_retry_with_linear_backoff() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let backoff = LinearBackoff::new(1, 2, 50);
        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction_with_retry(backoff, 3, |(tx_tree1,)| {
                let value = TestValue::alice();
                tx_tree1.insert(&1, &value)?;
                Ok(())
            });

        assert!(result.is_ok());
        assert!(tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_retry_with_constant_backoff() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let backoff = ConstantBackoff::new(5);
        let result: TransactionResult<(), crate::error::Error> =
            (&tree1,).transaction_with_retry(backoff, 2, |(tx_tree1,)| {
                let value = TestValue::alice();
                tx_tree1.insert(&1, &value)?;
                Ok(())
            });

        assert!(result.is_ok());
        assert!(tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_retry_respects_max_retries_on_abort() {
        let db = create_test_db().unwrap();
        let tree1 = db.get_tree::<TestSchema1>().unwrap();

        let backoff = ConstantBackoff::new(1);
        let result: TransactionResult<(), &'static str> =
            (&tree1,).transaction_with_retry(backoff, 3, |(tx_tree1,)| {
                let _ = tx_tree1.insert(&1, &TestValue::alice());
                Err(sled::transaction::ConflictableTransactionError::Abort(
                    "intentional abort",
                ))
            });

        // Should immediately fail on abort, not retry
        assert!(result.is_err());
        assert!(matches!(result, Err(TransactionError::Abort(_))));

        // Data should not be persisted due to abort
        assert!(!tree1.contains_key(&1).unwrap());
    }

    #[test]
    fn test_backoff_strategies() {
        let exp_backoff = ExponentialBackoff::new(10, 2.0, 1000);
        assert_eq!(exp_backoff.base_delay_ms(), 10);
        assert_eq!(exp_backoff.next_delay_ms(10), 20);
        assert_eq!(exp_backoff.next_delay_ms(500), 1000); // capped at max

        let linear_backoff = LinearBackoff::new(5, 3, 50);
        assert_eq!(linear_backoff.base_delay_ms(), 5);
        assert_eq!(linear_backoff.next_delay_ms(5), 8);
        assert_eq!(linear_backoff.next_delay_ms(48), 50); // capped at max

        let const_backoff = ConstantBackoff::new(100);
        assert_eq!(const_backoff.base_delay_ms(), 100);
        assert_eq!(const_backoff.next_delay_ms(100), 100);
        assert_eq!(const_backoff.next_delay_ms(999), 100);
    }

    #[test]
    fn test_default_backoff_strategies() {
        let exp_default = ExponentialBackoff::default();
        assert_eq!(exp_default.base_delay_ms(), 10);
        assert_eq!(exp_default.multiplier, 2.0);
        assert_eq!(exp_default.max_delay_ms, 5000);

        let linear_default = LinearBackoff::default();
        assert_eq!(linear_default.base_delay_ms(), 10);
        assert_eq!(linear_default.increment_ms, 10);
        assert_eq!(linear_default.max_delay_ms, 1000);

        let const_default = ConstantBackoff::default();
        assert_eq!(const_default.delay_ms, 100);
    }
}

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
impl<S1: Schema> SledTransactional for (SledTree<S1>,) {
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

macro_rules! impl_sled_transactional {
    ($(($idx:tt, $schema:ident, $var:ident)),+) => {
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

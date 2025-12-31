//! Primitive data types related to the bridge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_primitives::crypto::EvenPublicKey;

use crate::OperatorIdx;

// A table that maps [`OperatorIdx`] to the corresponding [`EvenPublicKey`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublickeyTable(pub BTreeMap<OperatorIdx, EvenPublicKey>);

impl From<BTreeMap<OperatorIdx, EvenPublicKey>> for PublickeyTable {
    fn from(value: BTreeMap<OperatorIdx, EvenPublicKey>) -> Self {
        Self(value)
    }
}

impl From<PublickeyTable> for Vec<EvenPublicKey> {
    fn from(value: PublickeyTable) -> Self {
        value.0.values().copied().collect()
    }
}

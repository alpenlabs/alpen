use std::fmt::Debug;

use crate::codec::{KeyCodec, ValueCodec};

/// A wrapper for `&'static str` for type safety.
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct TreeName(pub &'static str);

impl TreeName {
    pub fn into_inner(self) -> &'static str {
        self.0
    }
}

impl From<&'static str> for TreeName {
    fn from(value: &'static str) -> Self {
        Self(value)
    }
}

pub trait Schema: Debug + Send + Sync + Sized {
    const TREE_NAME: TreeName;

    type Key: KeyCodec<Self>;
    type Value: ValueCodec<Self>;
}

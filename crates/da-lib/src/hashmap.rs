use std::{collections::HashMap, hash::Hash};

use crate::Diff;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HashMapDiff<K, V> {
    #[default]
    None,
    Insert(K, V),
    Remove(K),
    Update(K, V), // Same as Insert ??
}

impl<K: Eq + Hash + Clone, V: Clone> Diff for HashMapDiff<K, V> {
    type Target = HashMap<K, V>;

    fn is_default(&self) -> bool {
        matches!(self, HashMapDiff::None)
    }

    fn apply(&self, source: &mut Self::Target) -> Result<(), crate::ApplyError> {
        match self {
            HashMapDiff::None => Ok(()),
            HashMapDiff::Insert(key, value) => {
                source.insert(key.clone(), value.clone());
                Ok(())
            }
            HashMapDiff::Remove(key) => {
                source.remove(key);
                Ok(())
            }
            HashMapDiff::Update(key, value) => {
                source.insert(key.clone(), value.clone());
                Ok(())
            }
        }
    }
}

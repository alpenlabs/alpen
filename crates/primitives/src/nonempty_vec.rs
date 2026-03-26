use std::ops::Deref;

use borsh::{BorshDeserialize, BorshSerialize};

/// A variant of `Vec` where non-emptyness is always guaranteed.
/// This exposes all read-only methods and only the safe methods(no removals) as `Vec`.
/// To use unsafe methods that break non-emptyness, use `to_vec`;
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    pub fn new(head: T) -> Self {
        Self(vec![head])
    }

    pub fn try_from_vec(v: Vec<T>) -> Result<Self, Vec<T>> {
        if v.is_empty() {
            Err(v)
        } else {
            Ok(Self(v))
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Non-empty vec version of `first` method to keep compatibility with `Vec`'s `first`.
    pub fn ensured_first(&self) -> &T {
        &self.0[0]
    }

    /// Non-empty vec version of `last` method to keep compatibility with `Vec`'s `last`.
    pub fn ensured_last(&self) -> &T {
        self.0
            .last()
            .expect("last(): non-empty vec should have at least an element")
    }

    pub fn push(&mut self, x: T) {
        self.0.push(x)
    }

    pub fn insert(&mut self, index: usize, value: T) {
        self.0.insert(index, value)
    }

    pub fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.0.extend(iter)
    }

    pub fn append(&mut self, other: &mut Vec<T>) {
        self.0.append(other)
    }

    pub fn to_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T> Deref for NonEmptyVec<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

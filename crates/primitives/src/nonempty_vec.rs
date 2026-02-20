use std::ops::Deref;

/// A variant of `Vec` where non-emptyness is always guaranteed.
/// This exposes all read-only methods and only the safe methods(no removals) as `Vec`.
/// To use unsafe methods that break non-emptyness, use `to_vec`;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    pub fn new(head: T, mut tail: Vec<T>) -> Self {
        tail.insert(0, head);
        Self(tail)
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

    pub fn first(&self) -> &T {
        &self.0[0]
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

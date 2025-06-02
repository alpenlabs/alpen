use std::{cmp::Ordering, mem};

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("provided vec was unsorted")]
    Unsorted,

    #[error("provided vec had duplicates")]
    Duplicates,
}

/// A vector wrapper that ensures the elements are sorted.
///
/// This *CAN* have duplicate entries.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize)]
pub struct SortedVec<T> {
    // Since it can have duplicate entries, we can't make it a wrapper around
    // `FlatTable`.
    inner: Vec<T>,
}

impl<T> SortedVec<T> {
    /// Constructs a new instance without validating the contents.
    ///
    /// NOTE: You _must_ ensure that the contents are already sorted.
    pub fn new_unchecked(inner: Vec<T>) -> Self {
        Self { inner }
    }

    /// Creates a new, empty [`SortedVec`].
    pub fn new_empty() -> Self {
        Self::new_unchecked(Vec::new())
    }

    /// Creates a new, empty [`SortedVec`] with given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new_unchecked(Vec::with_capacity(capacity))
    }

    /// Returns the number of elements in the [`SortedVec`].
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the [`SortedVec`] is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Exposes `iter` method of the inner vector
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.inner.iter()
    }

    /// Consumes the [`SortedVec`] and returns the inner [`Vec`].
    pub fn into_inner(self) -> Vec<T> {
        self.inner
    }

    /// Returns a slice of the inner vec.
    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }
}

impl<T: Ord + Clone> SortedVec<T> {
    /// Creates a [`SortedVec`] from an arbitrary [`Vec`], sorting the elements
    /// unconditionally.
    pub fn from_unsorted(mut vec: Vec<T>) -> Self {
        vec.sort();
        Self::new_unchecked(vec)
    }

    /// Finds the index of a value.  If it's present, returns `Ok` of the index,
    /// if not present, returns `Err` of the index it would be present.
    ///
    /// This is implemented by doing a binary search.
    ///
    /// NOTE: If there are multiple matches, there is no guarantee which will be used,
    /// and this may not be consistent.
    pub fn find_index(&self, value: &T) -> Result<usize, usize> {
        self.inner.binary_search(value)
    }

    /// Returns a reference to the element at the given index.
    pub fn get_index(&self, index: usize) -> Option<&T> {
        self.inner.get(index)
    }

    /// Inserts an element into the [`SortedVec`], maintaining sorted order, and
    /// allowing for inserting duplicate entries.
    ///
    /// This runs in O(n) because of shifting of elements.
    pub fn insert(&mut self, value: T) {
        let pos = self.inner.binary_search(&value).unwrap_or_else(|e| e);
        self.inner.insert(pos, value);
    }

    /// Removes an element from the [`SortedVec`]. Returns `true` if the element was found and
    /// removed. This runs in O(n) due to shifting of elements.
    ///
    /// NOTE: If multiple matches exist, only one will be removed.
    pub fn remove(&mut self, value: &T) -> bool {
        if let Ok(pos) = self.find_index(value) {
            self.inner.remove(pos);
            true
        } else {
            false
        }
    }

    /// Checks if the [`SortedVec`] contains the given value.
    pub fn contains(&self, value: &T) -> bool {
        self.find_index(value).is_ok()
    }
}

impl<T: Ord + Clone> SortedVec<T> {
    /// Merge another [`SortedVec`]
    pub fn merge(&mut self, other: &Self) {
        let mut merged: Vec<T> = Vec::with_capacity(self.len() + other.len());
        let mut i = 0;
        let mut j = 0;
        while i < self.len() && j < other.len() {
            match self.inner[i].cmp(&other.inner[j]) {
                Ordering::Greater => {
                    merged.push(other.inner[j].clone());
                    j += 1;
                }
                _ => {
                    merged.push(self.inner[i].clone());
                    i += 1;
                }
            }
        }

        // Append remaining elements
        merged.extend_from_slice(&self.inner[i..]);
        merged.extend_from_slice(&other.inner[j..]);

        self.inner = merged;
    }
}

impl<T: Clone> SortedVec<T> {
    /// Converts to vec
    pub fn to_vec(&self) -> Vec<T> {
        self.inner.clone()
    }
}

impl<T> Default for SortedVec<T> {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl<T: Ord> TryFrom<Vec<T>> for SortedVec<T> {
    type Error = Error;

    /// If the value provided is not sorted, returns an error.
    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        if value.is_sorted() {
            Ok(Self::new_unchecked(value))
        } else {
            Err(Error::Unsorted)
        }
    }
}

/// Extra implementation logic that ensures that the deserialized vec is
/// sorted.  Does not sort it itself, instead it errors.
impl<T: Ord + BorshDeserialize> BorshDeserialize for SortedVec<T> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let vec = <Vec<T> as BorshDeserialize>::deserialize_reader(reader)?;
        Self::try_from(vec)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "vec unsorted"))
    }
}

/// Implemented by entries in a [`FlatTable`].  This is an intrusive
/// collection entry, so the sorted key is a member of the entry type.
pub trait TableEntry {
    /// The key used to sort the entries on.
    type Key: Ord;

    /// Gets the ref to the key in the entry.
    fn get_key(&self) -> &Self::Key;
}

/// Sorts a slice of [`TableEntry`]s according to their natural sorting.  Does not
/// check for duplicates, but preserves the order of equally keyed elements.
fn sort_entry_slice<T: TableEntry>(vec: &mut [T]) {
    vec.sort_by(|l, r| Ord::cmp(l.get_key(), r.get_key()));
}

/// Checks to see if there are any duplicate keys in a sorted vec of
/// [`TableEntry`]s.  This only works if it's already sorted, not well-defined
/// if not.
fn check_duplicate_keys<T: TableEntry>(v: &[T]) -> bool {
    v.windows(2)
        .any(|pair| pair[0].get_key() == pair[1].get_key())
}

/// Describes the ordering status of some slice that we're interpreting as a
/// sorted table.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum TableState {
    /// Sorted and no duplicates.
    Safe,

    /// Sorted, but duplicates.
    Duplicates,

    /// Not properly sorted.
    Unsorted,
}

/// Checks the "safety" of a slice that we'd want to convert into a
/// [`FlatTable`].
fn check_table_vec<T: TableEntry>(v: &[T]) -> TableState {
    let mut dups = false;

    for pair in v.windows(2) {
        match Ord::cmp(pair[0].get_key(), pair[1].get_key()) {
            // "less than" is the normal, happy case
            Ordering::Less => {}
            Ordering::Equal => {
                dups = true;
            }
            Ordering::Greater => return TableState::Unsorted,
        }
    }

    if dups {
        TableState::Duplicates
    } else {
        TableState::Safe
    }
}

/// A flat lookup table.  This is an intrusive collection.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct FlatTable<T: TableEntry> {
    inner: Vec<T>,
}

impl<T: TableEntry> FlatTable<T> {
    /// Creates a new instance by wrapping a vec without checking that it's
    /// sorted.
    pub fn new_unchecked(inner: Vec<T>) -> Self {
        Self { inner }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new_unchecked(Vec::new())
    }

    /// Creates a new empty instance with some preallocated capacity in the
    /// underlying vec.
    pub fn with_capacity(n: usize) -> Self {
        Self::new_unchecked(Vec::with_capacity(n))
    }

    /// Creates a new instance from an existing vec, sorting it unconditionally.
    ///
    /// If there duplicates after sorting, returns error.
    pub fn try_from_unsorted(mut vec: Vec<T>) -> Result<Self, Error> {
        sort_entry_slice(&mut vec);
        if !check_duplicate_keys(&vec) {
            Ok(Self::new_unchecked(vec))
        } else {
            Err(Error::Duplicates)
        }
    }

    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn into_vec(self) -> Vec<T> {
        self.inner
    }

    /// Finds the index of the entry with some key.  If it's present, returns
    /// `Ok` of the index, if not present, returns `Err` of the index it would
    /// be present.
    ///
    /// This is implemented by doing a binary search.
    pub fn find_index(&self, k: &T::Key) -> Result<usize, usize> {
        // This is apparently how you do the call for this.
        self.inner.binary_search_by(|probe| probe.get_key().cmp(k))
    }

    /// Checks if the table contains the provided key.
    pub fn contains(&self, k: &T::Key) -> bool {
        self.find_index(k).is_ok()
    }

    /// Gets the entry with some key.
    pub fn get(&self, k: &T::Key) -> Option<&T> {
        self.find_index(k).ok().map(|i| &self.inner[i])
    }

    /// Inserts an entry into the table.  If this overwrites an existing entry,
    /// returns that previous value.
    pub fn insert(&mut self, mut e: T) -> Option<T> {
        match self.find_index(e.get_key()) {
            Ok(i) => {
                // This does an in-place swap, so we actually return the arg
                // that we overwrote.
                let entry = self
                    .inner
                    .get_mut(i)
                    .expect("sortedvec: missing expected index");
                mem::swap(entry, &mut e);
                Some(e)
            }
            Err(i) => {
                self.inner.insert(i, e);
                None
            }
        }
    }

    /// Removes an entry if it exists.
    pub fn remove(&mut self, k: &T::Key) -> Option<T> {
        Some(self.inner.remove(self.find_index(k).ok()?))
    }

    /// Iterates over the entries in order from first to last.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.inner.iter()
    }

    /// Consumes another instance, merging its elements into this one,
    /// overwriting where there are conflicts.
    ///
    /// This performs an allocation that's as large as sum of the lengths of the
    /// two lists, and does not shrink it afterwards.
    pub fn merge(&mut self, other: Self) {
        // Do some shenanigans to minimize number of `Vec` instances
        // constructed.
        let mut merged = Vec::with_capacity(self.inner.len() + other.inner.len());
        mem::swap(&mut merged, &mut self.inner);
        let old_inner = merged;
        let mut iter_self = old_inner.into_iter().peekable();

        let mut iter_other = other.inner.into_iter().peekable();

        let merged = &mut self.inner; // this is the real target
        while iter_self.peek().is_some() || iter_other.peek().is_some() {
            match (iter_self.peek(), iter_other.peek()) {
                // In the usual case, we compare the two front elements and take
                // just the "lesser" one.  Unless they're equal, then we take
                // the new one and discard the old one.
                (Some(a), Some(b)) => match Ord::cmp(a.get_key(), b.get_key()) {
                    Ordering::Less => merged.push(iter_self.next().unwrap()),
                    Ordering::Greater => merged.push(iter_other.next().unwrap()),
                    Ordering::Equal => {
                        // Discard "self", accept "other".
                        iter_self.next();
                        merged.push(iter_other.next().unwrap());
                    }
                },

                // In this case, there's no more "other" elements so we can just
                // extend from what's left here.
                (Some(_), None) => {
                    merged.extend(iter_self);
                    break;
                }

                // In this case, there's no more "self" elements so we can again
                // just extend from what's left there.
                (None, Some(_)) => {
                    merged.extend(iter_other);
                    break;
                }

                // In this case they terminated at the same time, nice.
                (None, None) => break,
            }
        }
    }
}

impl<T: TableEntry> TryFrom<Vec<T>> for FlatTable<T> {
    /// Error returned if there are duplicate entries.
    type Error = Error;

    /// Tries to construct an instance of [`FlatTable`] from a vec of entries.
    ///
    /// Fails if there are any duplicate entries in the vec.
    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        match check_table_vec(&value) {
            TableState::Duplicates => Err(Error::Duplicates),
            TableState::Unsorted => Err(Error::Unsorted),
            _ => Ok(Self::new_unchecked(value)),
        }
    }
}

/// Extra implementation logic that ensures that the deserialized vec is
/// sorted and has no duplicates.  Does not sort it itself, instead it errors.
impl<T: TableEntry + BorshDeserialize> BorshDeserialize for FlatTable<T> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let vec = <Vec<T> as BorshDeserialize>::deserialize_reader(reader)?;
        Self::try_from(vec).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "vec unsorted or has duplicates",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sv_insert_and_sorted_order() {
        let mut sorted_vec = SortedVec::new_empty();
        sorted_vec.insert(5);
        sorted_vec.insert(3);
        sorted_vec.insert(8);

        assert_eq!(&sorted_vec.to_vec(), &[3, 5, 8]); // Check the sorted order
    }

    #[test]
    fn test_sv_remove_existing() {
        let mut sorted_vec = SortedVec::from_unsorted(vec![3, 5, 8]);
        let removed = sorted_vec.remove(&5);

        assert!(removed);
        assert_eq!(&sorted_vec.into_inner(), &[3, 8]); // Check the sorted order after removal
    }

    #[test]
    fn test_sv_remove_non_existing() {
        let mut sorted_vec = SortedVec::from_unsorted(vec![3, 5, 8]);
        let removed = sorted_vec.remove(&10);

        assert!(!removed); // Removing a non-existing element
        assert_eq!(sorted_vec.into_inner(), &[3, 5, 8]);
    }

    #[test]
    fn test_sv_contains() {
        let sorted_vec = SortedVec::from_unsorted(vec![3, 5, 8]);

        assert!(sorted_vec.contains(&5));
        assert!(!sorted_vec.contains(&10));
    }

    #[test]
    fn test_sv_len_and_empty() {
        let mut sorted_vec = SortedVec::new_empty();

        assert!(sorted_vec.is_empty());
        assert_eq!(sorted_vec.len(), 0);

        sorted_vec.insert(5);
        assert!(!sorted_vec.is_empty());
        assert_eq!(sorted_vec.len(), 1);
    }

    #[test]
    fn test_sv_merge() {
        // Create vectors with duplicate elements
        let mut sv1 = SortedVec::new_empty();
        sv1.insert(1);
        sv1.insert(3);
        sv1.insert(4);
        sv1.insert(4);
        sv1.insert(5);

        let mut sv2 = SortedVec::new_empty();
        sv2.insert(3);
        sv2.insert(5);
        sv2.insert(7);
        sv2.insert(8);

        sv1.merge(&sv2);

        assert_eq!(sv1.as_slice(), &[1, 3, 3, 4, 4, 5, 5, 7, 8]);
    }

    #[allow(unused)]
    struct Pair(u32, u32);

    impl TableEntry for Pair {
        type Key = u32;
        fn get_key(&self) -> &Self::Key {
            &self.0
        }
    }

    #[test]
    fn test_ft_check_table_vec_safe() {
        let vec = vec![Pair(0, 2), Pair(10, 3), Pair(20, 4)];
        assert_eq!(check_table_vec(&vec), TableState::Safe);
    }

    #[test]
    fn test_ft_check_table_vec_duplicates() {
        let vec = vec![Pair(0, 2), Pair(0, 3), Pair(20, 4)];
        assert_eq!(check_table_vec(&vec), TableState::Duplicates);
    }

    #[test]
    fn test_ft_check_table_vec_unsorted() {
        let vec = vec![Pair(10, 2), Pair(0, 3), Pair(20, 4)];
        assert_eq!(check_table_vec(&vec), TableState::Unsorted);
    }
}

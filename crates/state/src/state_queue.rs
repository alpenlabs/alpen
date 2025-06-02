//! Generic queue type used in CL state, serializable with Borsh.
//!
//! There's a few different types of entries in CL state that represent "queues"
//! of data that we push things into, have monotonic indexes, and pull out.
//! This module is meant to implement this bookkeeping generically.  This does
//! *not* use `VecDeque` internally, as it's designed around being easily
//! serializable.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct StateQueue<T> {
    /// The front of the queue that we take entries out of.
    base_idx: u64,

    /// The entries in the queue.
    entries: Vec<T>,
}

impl<T> StateQueue<T> {
    /// Creates a new empty fresh queue.
    pub const fn new_empty() -> Self {
        Self {
            base_idx: 0,
            entries: Vec::new(),
        }
    }

    /// Creates a new empty queue with a particular base offset.
    pub const fn new_at_index(base_idx: u64) -> Self {
        Self {
            base_idx,
            entries: Vec::new(),
        }
    }

    /// Returns the "base index", which is the absolute position of the front of
    /// the queue, even if there is no element at the front of the queue.
    pub const fn base_idx(&self) -> u64 {
        self.base_idx
    }

    /// Returns a slice over the entries in the queue, without their positioning
    /// information.  Consider if `.iter_entries` is more well-suited.
    // TODO is it bad to expose this?
    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Returns the number of items in the queue.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the offset index in the entries list for an absolute index.
    /// This does not fail for out of bounds values or overflow.
    ///
    /// Not meant for public consumption, the backing entries array is meant to
    /// be internal.
    const fn get_off_for_abs(&self, idx: u64) -> u64 {
        idx - self.base_idx
    }

    /// Returns the index of the element at the front of the queue, if there is
    /// one.  This is semantically distinct from `.base_idx()`, but if this
    /// returns `Some` then that will return the same value.
    pub fn front_idx(&self) -> Option<u64> {
        if !self.entries.is_empty() {
            Some(self.base_idx)
        } else {
            None
        }
    }

    /// Returns the absolute index of and a reference to the front entry in the
    /// queue, if it exists.
    pub fn front_entry(&self) -> Option<(u64, &T)> {
        self.entries.first().map(|e| (self.base_idx, e))
    }

    /// Returns a reference to the front entry of the queue, if it exists.
    pub fn front(&self) -> Option<&T> {
        self.entries.first()
    }

    /// Returns a mut ref to the front entry in the queue, if it exists.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.entries.last_mut()
    }

    /// Returns the index of the element at the back of the queue, if there is
    /// one.
    pub fn back_idx(&self) -> Option<u64> {
        self.front_idx().map(|i| i + self.entries.len() as u64 - 1)
    }

    /// Returns the absolute index of and a reference to the back entry in the
    /// queue, if it exists.
    pub fn back_entry(&self) -> Option<(u64, &T)> {
        // Is there a better way to do this?
        self.entries.last().map(|e| (self.back_idx().unwrap(), e))
    }

    /// Returns a reference to the back entry in the queue, if it exists.
    pub fn back(&self) -> Option<&T> {
        self.entries.last()
    }

    /// Returns a mut ref to the back entry in the queue, if it exists.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.entries.last_mut()
    }

    /// Returns the absolute index of the next element to be written to the queue.
    pub fn next_idx(&self) -> u64 {
        self.base_idx + self.entries.len() as u64
    }

    /// Gets an entry by absolute position.
    ///
    /// This does the math to calculate the correct offset.
    pub fn get_absolute(&self, idx: u64) -> Option<&T> {
        if idx < self.base_idx || idx >= self.next_idx() {
            return None;
        }

        Some(
            self.entries
                .get(self.get_off_for_abs(idx) as usize)
                .unwrap(),
        )
    }

    /// Checks if the queue contains an element with the provided absolute index.
    pub fn contains_abs(&self, idx: u64) -> bool {
        let end = self.base_idx + self.len() as u64;
        idx >= self.base_idx && idx < end
    }

    /// Pushes an entry to the back of the queue, returning the absolute
    /// position of the entry.
    pub fn push_back(&mut self, ent: T) -> u64 {
        let idx = self.next_idx();
        self.entries.push(ent);
        idx
    }

    /// Pops an entry from the back of the queue.
    pub fn pop_back(&mut self) -> Option<T> {
        self.entries.pop()
    }

    /// Pops the front of the queue.
    pub fn pop_front(&mut self) -> Option<T> {
        if !self.entries.is_empty() {
            self.base_idx += 1;
            Some(self.entries.remove(0))
        } else {
            None
        }
    }

    /// Pops the front `n` elements from the queue, only if they can all be popped.
    ///
    /// This is a batch operation so that we don't have to repeatedly take
    /// single elements out of the vec with `.pop_front()` and move the later
    /// ones over if we want to take out multiple elements
    ///
    /// Returns `None` if there are not enough elements to pop.
    pub fn pop_front_n_vec(&mut self, n: usize) -> Option<Vec<T>> {
        if self.entries.len() < n {
            return None;
        }

        // Split the queue entries we want to keep into its own vec.
        let out = {
            let mut new_entries = self.entries.split_off(n);
            std::mem::swap(&mut self.entries, &mut new_entries);
            new_entries
        };

        // Now adjust the base index.
        self.base_idx += n as u64;
        Some(out)
    }

    /// Pops the front N element of the queue, only if they can be poppped
    /// N is const, because this uses compile time guarantees to convert
    ///
    /// This is a batch operation so that we don't have to repeatedly take
    /// single elements out of the vec with `.pop_front()` and move the later
    /// ones over if we want to take out multiple elements, and it provides a
    /// check to ensure that it can really be done as a single operation.
    pub fn pop_front_n_arr<const N: usize>(&mut self) -> Option<[T; N]> {
        if self.entries.len() < N {
            return None;
        }

        // Split the queue entries we want to keep into its own vec.
        let out = {
            let mut new_entries = self.entries.split_off(N);
            std::mem::swap(&mut self.entries, &mut new_entries);
            new_entries
        };

        // TODO verify that this is safe
        let slice_box = out.into_boxed_slice();
        let slice_ptr = slice_box.as_ptr();
        assert_eq!(slice_box.len(), N);
        let arr_box = unsafe {
            std::mem::forget(slice_box);
            Box::<[T; N]>::from_raw(slice_ptr as *mut [T; N])
        };

        // Now just copy it to the stack.
        self.base_idx += N as u64;
        Some(*arr_box)
    }

    /// Drops n elements from the front of the queue.  Will not drop more
    /// elements than there are in the queue.  Returns if successful or not.
    pub fn drop_n(&mut self, n: u64) -> bool {
        let nn = n as usize;
        if nn > self.entries.len() {
            return false;
        }

        assert_eq!(
            self.entries.drain(..nn).count(),
            nn,
            "statequeue: inconsistent drain"
        );
        self.base_idx += n;

        true
    }

    /// Drops some number of elements to make the provided index be the new base
    /// of the queue.  Returns if successful or not.
    pub fn drop_abs(&mut self, new_base: u64) -> bool {
        if new_base < self.base_idx {
            return false;
        }

        let diff = new_base - self.base_idx;
        self.drop_n(diff)
    }

    /// Drops recent elements from the queue, making the specified absolute
    /// index the new "next element".  Passing in `.next_idx()` successfully
    /// does no operation.
    ///
    /// Returns true if successful.  Returns false if not possible, making no
    /// changes.
    pub fn truncate_abs(&mut self, new_next_idx: u64) -> bool {
        if new_next_idx < self.base_idx || new_next_idx > self.next_idx() {
            return false;
        }

        let new_len = new_next_idx - self.base_idx;
        self.entries.truncate(new_len as usize);
        true
    }

    /// Iterates over the entries in the queue, from front to back.
    pub fn iter_entries(&self) -> impl Iterator<Item = (u64, &'_ T)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + self.base_idx, e))
    }
}

#[cfg(test)]
mod tests {
    use super::StateQueue;

    // TODO maybe add a queue that goes back and forth several times
    #[test]
    fn test_push_pop_arr() {
        let mut q = StateQueue::<u64>::new_empty();

        let i0 = q.push_back(5);
        assert_eq!(i0, 0);
        let i1 = q.push_back(6);
        assert_eq!(i1, 1);
        q.push_back(7);
        q.push_back(8);
        q.push_back(9);
        let i2a = q.push_back(10);
        assert_eq!(q.pop_back(), Some(10));
        let i2b = q.push_back(15);
        assert_eq!(i2a, i2b);
        q.push_back(20);
        q.push_back(25);

        eprintln!("queue: {q:?}");

        let n0 = q.next_idx();
        assert_eq!(n0, 8);

        let arr = q.pop_front_n_arr::<3>();
        assert_eq!(arr, Some([5, 6, 7]));
        let b0 = q.base_idx();
        assert_eq!(b0, 3);

        let n1 = q.next_idx();
        assert_eq!(n1, 8);

        let a0 = q.pop_front();
        assert_eq!(a0, Some(8));
        let b1 = q.base_idx();
        assert_eq!(b1, 4);

        let n2 = q.next_idx();
        assert_eq!(n2, 8);

        let arr2 = q.pop_front_n_arr::<10>();
        assert_eq!(arr2, None);
        let arr3 = q.pop_front_n_arr::<4>();
        assert_eq!(arr3, Some([9, 15, 20, 25]));

        let to_pop_len = 4;
        q.push_back(10);
        q.push_back(11);
        q.push_back(12);
        q.push_back(13);
        let vec1 = q.pop_front_n_vec(to_pop_len);
        assert_eq!(vec1, Some(vec![10, 11, 12, 13]));

        let n3 = q.next_idx();
        assert_eq!(n3, 12);

        assert!(q.is_empty());
        let b2 = q.base_idx();
        assert_eq!(b2, 12);
    }

    #[test]
    fn test_truncate() {
        let mut q = StateQueue::<u64>::new_at_index(10);

        q.push_back(5); // idx = 10
        q.push_back(6);
        q.push_back(7);
        q.push_back(8); // idx = 13

        assert_eq!(q.back_idx(), Some(13));
        assert_eq!(q.back(), Some(&8));

        assert!(q.truncate_abs(13));
        assert_eq!(q.back_idx(), Some(12));
        assert_eq!(q.back(), Some(&7));

        assert!(q.truncate_abs(11));
        assert_eq!(q.back_idx(), Some(10));
        assert_eq!(q.back(), Some(&5));

        // TODO
    }
}

//! Queue DA pattern.

use crate::{DaRegister, DaWrite};

// TODO make this generic over the next type

/// Provides the interface for a Queue DA write to update a type.
pub trait DaQueueTarget {
    /// Queue entry type.
    type Entry;

    /// Inserts one or more entries into the back of the queue, in order.
    fn insert_entries(&mut self, entries: &[Self::Entry]);

    /// Updates the index of the front of the queue.
    fn update_front(&mut self, idx: u32);
}

#[derive(Clone, Debug)]
pub struct DaQueue<Q: DaQueueTarget> {
    /// New entries to be appended to the back.
    tail: Vec<Q::Entry>,

    /// The new front of the queue.
    // TODO should this be converted to a counter?
    new_front: DaRegister<u32>,
}

impl<Q: DaQueueTarget> DaQueue<Q> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<Q: DaQueueTarget> Default for DaQueue<Q> {
    fn default() -> Self {
        Self {
            tail: Vec::new(),
            new_front: DaRegister::new_unset(),
        }
    }
}

impl<Q: DaQueueTarget> DaWrite for DaQueue<Q> {
    type Target = Q;

    fn is_default(&self) -> bool {
        self.tail.is_empty() && self.new_front.is_default()
    }

    fn apply(&self, target: &mut Self::Target) {
        target.insert_entries(&self.tail);
        if let Some(v) = self.new_front.new_value() {
            target.update_front(*v);
        }
    }
}

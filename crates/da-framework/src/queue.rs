//! Queue DA pattern.

use crate::{
    BuilderError, Codec, CodecResult, CompoundMember, DaBuilder, DaWrite, Decoder, Encoder,
};

// TODO make this generic over the queue type

/// The type that we increment the front by.
pub type IncrTy = u16;

/// The type that we describe length of new tail entries with.
pub type TailLenTy = u16;

/// Type of the head word.
pub type HeadTy = u16;

/// The mask for the increment portion of the head word.
const HEAD_WORD_INCR_MASK: u16 = 0x7fff;

/// Bits we shift the tail flag bit by.
const TAIL_BIT_SHIFT: u16 = IncrTy::MAX - 1;

/// Provides the interface for a Queue DA write to update a type.
pub trait DaQueueTarget {
    /// Queue entry type.
    type Entry: Codec;

    /// Gets the global index of the next entry to be removed from the queue.
    fn cur_front(&self) -> IncrTy; // TODO make a `IdxTy`

    /// Gets what would be the global index of the next entry to be added to the
    /// queue.
    fn cur_next(&self) -> IncrTy; // TODO make a `IdxTy`

    /// Inserts one or more entries into the back of the queue, in order.
    fn insert_entries(&mut self, entries: &[Self::Entry]);

    /// Increments the index of the front of the queue.
    fn increment_front(&mut self, incr: IncrTy);
}

#[derive(Clone, Debug)]
pub struct DaQueue<Q: DaQueueTarget> {
    /// New entries to be appended to the back.
    tail: Vec<Q::Entry>,

    /// The new front of the queue.
    // TODO should this be converted to a counter?
    incr_front: IncrTy,
}

impl<Q: DaQueueTarget> DaQueue<Q> {
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    // TODO add fn to safely add to the back, needs some context
}

impl<Q: DaQueueTarget> Default for DaQueue<Q> {
    fn default() -> Self {
        Self {
            tail: Vec::new(),
            incr_front: 0,
        }
    }
}

impl<Q: DaQueueTarget> DaWrite for DaQueue<Q> {
    type Target = Q;

    fn is_default(&self) -> bool {
        self.tail.is_empty() && self.incr_front == 0
    }

    fn apply(&self, target: &mut Self::Target) {
        target.insert_entries(&self.tail);
        if self.incr_front > 0 {
            target.increment_front(self.incr_front);
        }
    }
}

impl<Q: DaQueueTarget> CompoundMember for DaQueue<Q> {
    fn default() -> Self {
        <Self as Default>::default()
    }

    fn is_default(&self) -> bool {
        <Self as DaWrite>::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        let head = IncrTy::decode(dec)?;
        let (is_tail_entries, incr_front) = decode_head(head);

        let mut tail = Vec::new();

        if is_tail_entries {
            let tail_len = TailLenTy::decode(dec)?;
            for _ in 0..tail_len {
                let e = <Q::Entry as Codec>::decode(dec)?;
                tail.push(e);
            }
        }

        Ok(Self { incr_front, tail })
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        let is_tail_entries = !self.tail.is_empty();
        let head = encode_head(is_tail_entries, self.incr_front);
        head.encode(enc)?;

        if is_tail_entries {
            let len_native = self.tail.len() as TailLenTy;
            len_native.encode(enc)?;

            for e in &self.tail {
                e.encode(enc)?;
            }
        }

        Ok(())
    }
}

/// Decodes the "head word".
///
/// The topmost bit is if there are new writes.  The remaining bits are the
/// increment to the index.
fn decode_head(v: HeadTy) -> (bool, IncrTy) {
    let incr = v & HEAD_WORD_INCR_MASK;
    let is_new_entries = (v >> TAIL_BIT_SHIFT) > 0;
    (is_new_entries, incr)
}

/// Encodes the "head word".
fn encode_head(new_entries: bool, v: IncrTy) -> HeadTy {
    if v > HEAD_WORD_INCR_MASK {
        panic!("da/queue: tried to increment front by too much {v}");
    }

    ((new_entries as IncrTy) << TAIL_BIT_SHIFT) | (v & HEAD_WORD_INCR_MASK)
}

/// Builder for [`DaQueue`].
pub struct DaQueueBuilder<Q: DaQueueTarget> {
    // FIXME use `IdxTy`
    original_front_pos: IncrTy,
    new_front_pos: IncrTy,
    original_next_pos: IncrTy,
    new_entries: Vec<Q::Entry>,
}

impl<Q: DaQueueTarget> DaQueueBuilder<Q> {
    /// Returns what would be the idx of the next element to be added to the
    /// queue.
    pub fn next_idx(&self) -> usize {
        self.original_next_pos as usize + self.new_entries.len()
    }

    /// Tries to add to the increment to the front of the queue.
    ///
    /// Returns if successful, fails if overflow.
    pub fn add_front_incr(&mut self, incr: IncrTy) -> bool {
        // TODO add checks to only allow increments if there's entries
        let incr_front = self.new_front_pos - self.original_front_pos;
        let new_front = (self.new_front_pos as u64) + (incr as u64);

        // So we don't overrun the back of the entries that'd be added.
        if new_front >= self.next_idx() as u64 {
            return false;
        }

        let new_incr = (incr as u64) + (incr_front as u64);
        if new_incr >= HEAD_WORD_INCR_MASK as u64 {
            false
        } else {
            self.new_front_pos = new_front as IncrTy;
            true
        }
    }

    /// Appends an entry to the queue.
    pub fn append_entry(&mut self, e: Q::Entry) {
        // FIXME this doesn't account for bounds checks
        self.new_entries.push(e);
    }

    /// Returns the count of new entries we added that would be consumed.
    fn consumed_new_entries(&self) -> usize {
        // FIXME this doesn't account for bounds checks
        let new_front = self.new_front_pos as i64;
        let orig_next = self.original_next_pos as i64;
        let overrun = new_front - orig_next;
        (overrun as usize).clamp(0, usize::MAX)
    }
}

impl<Q: DaQueueTarget> DaBuilder<Q> for DaQueueBuilder<Q> {
    type Write = DaQueue<Q>;

    fn from_source(t: Q) -> Self {
        Self {
            original_front_pos: t.cur_front(),
            new_front_pos: t.cur_front(),
            original_next_pos: t.cur_next(),
            new_entries: Vec::new(),
        }
    }

    fn into_write(mut self) -> Result<Self::Write, BuilderError> {
        // Remove things from this that are redundant.
        self.new_entries.drain(..self.consumed_new_entries());

        let tail = self.new_entries;
        let incr_front = self.new_front_pos - self.original_front_pos;
        Ok(DaQueue { tail, incr_front })
    }
}

// TODO tests for the wacky logic up ^there

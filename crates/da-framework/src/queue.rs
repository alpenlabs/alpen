//! Queue DA pattern.

use crate::{
    Codec, CodecError, CodecResult, CompoundMember, DaRegister, DaWrite, Decoder, Encoder,
};

// TODO make this generic over the next type

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

    /// Tries to add to the increment to the front of the queue.
    ///
    /// Returns if successful, fails if overflow.
    pub fn add_front_incr(&mut self, incr: IncrTy) -> bool {
        let new_incr = (incr as u64) + (self.incr_front as u64);
        if new_incr >= HEAD_WORD_INCR_MASK as u64 {
            false
        } else {
            self.incr_front = new_incr as IncrTy;
            true
        }
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

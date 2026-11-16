//! Queue DA pattern.

use crate::{
    BuilderError, Codec, CodecError, CodecResult, CompoundMember, DaBuilder, DaQueueError, DaWrite,
    Decoder, Encoder,
};

// TODO(STR-2148): make this generic over the queue type

/// The type that we use to refer to queue indexes.
pub type IdxTy = usize;

/// The type that we increment the front by.
pub type IncrTy = u16;

/// The type that we describe length of new tail entries with.
pub type TailLenTy = u16;

/// Type of the head word.
pub type HeadTy = u16;

/// The mask for the increment portion of the head word.
const HEAD_WORD_INCR_MASK: u16 = 0x7fff;

/// Whether `added` entries appended at index `next` still fit the index type
/// (`IncrTy` for now, STR-2148). The builder and `apply` share this bound, so
/// a write the builder emits never fails to apply for it.
fn index_fits(next: u64, added: u64) -> bool {
    next + added <= IncrTy::MAX as u64
}

/// Bits we shift the tail flag bit by.
const TAIL_BIT_SHIFT: u32 = IncrTy::BITS - 1;

/// Provides the interface for a Queue DA write to update a type.
pub trait DaQueueTarget {
    /// Queue entry type.
    type Entry: Codec;

    /// Gets the global index of the next entry to be removed from the queue.
    ///
    /// Never past [`Self::cur_next`]: a target that reports otherwise is
    /// corrupt, and the builder asserts it while [`DaQueue::apply`] refuses it.
    fn cur_front(&self) -> IncrTy; // TODO(STR-2148): make a `IdxTy`

    /// Gets what would be the global index of the next entry to be added to the
    /// queue.
    fn cur_next(&self) -> IncrTy; // TODO(STR-2148): make a `IdxTy`

    /// Increments the index of the front of the queue.
    ///
    /// [`DaQueue::apply`] only calls this with an `incr` of at most the number
    /// of entries the queue holds once the write's tail is in, so the front
    /// never passes the next index.
    fn increment_front(&mut self, incr: IncrTy);

    /// Gets the entry at the given index, if it exists and hasn't been removed.
    ///
    /// Returns `None` if the index is not present in the queue.
    fn get(&self, idx: IdxTy) -> Option<&Self::Entry>;

    /// Inserts one or more entries into the back of the queue, in order.
    fn insert_entries(&mut self, entries: &[Self::Entry]);
}

#[derive(Clone, Debug)]
pub struct DaQueue<Q: DaQueueTarget> {
    /// New entries to be appended to the back.
    tail: Vec<Q::Entry>,

    /// The new front of the queue.
    // TODO(STR-2148): should this be converted to a counter?
    incr_front: IncrTy,
}

impl<Q: DaQueueTarget> DaQueue<Q> {
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    // TODO(STR-2148): add fn to safely add to the back, needs some context
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

    type Context = ();

    type Error = DaQueueError;

    fn is_default(&self) -> bool {
        self.tail.is_empty() && self.incr_front == 0
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), Self::Error> {
        let front = target.cur_front() as u64;
        let next = target.cur_next() as u64;
        if front > next {
            return Err(DaQueueError::FrontPastNext { front, next });
        }

        // Refuse a tail the index type cannot hold rather than let the
        // target's counters wrap.
        let added = self.tail.len() as u64;
        if !index_fits(next, added) {
            return Err(DaQueueError::IndexOverflow { next, added });
        }

        // A decoded write carries whatever increment was on the wire, so bound
        // it by the entries the queue holds once the tail is in, which is the
        // bound the builder enforces: an entry appended and consumed in the
        // same write stays in the tail so that every entry keeps the global
        // index the builder's view gave it. The front may reach the next index
        // (an empty queue) but not pass it.
        let incr = self.incr_front as u64;
        let len_with_tail = next - front + added;
        if incr > len_with_tail {
            return Err(DaQueueError::FrontOverrun {
                incr,
                len_with_tail,
            });
        }

        target.insert_entries(&self.tail);
        if self.incr_front > 0 {
            target.increment_front(self.incr_front);
        }
        Ok(())
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

        // `encode_set` sets the tail flag iff there are entries and never
        // writes the default queue (the compound clears the bit for it), so a
        // flag with no entries or a bare zero head would each give a write a
        // second spelling.
        if !is_tail_entries && incr_front == 0 {
            return Err(CodecError::InvalidVariant("queue"));
        }

        let mut tail = Vec::new();

        if is_tail_entries {
            let tail_len = TailLenTy::decode(dec)?;
            if tail_len == 0 {
                return Err(CodecError::InvalidVariant("queue"));
            }
            for _ in 0..tail_len {
                let e = <Q::Entry as Codec>::decode(dec)?;
                tail.push(e);
            }
        }

        Ok(Self { incr_front, tail })
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        // The default queue is written as a clear bit by the compound, never here.
        if <Self as DaWrite>::is_default(self) {
            return Err(CodecError::InvalidVariant("queue"));
        }
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
    original_front_pos: IdxTy,
    new_front_pos: IdxTy,
    original_next_pos: IdxTy,
    new_entries: Vec<Q::Entry>,
}

impl<Q: DaQueueTarget> DaQueueBuilder<Q> {
    /// Returns what would be the idx of the next element to be added to the
    /// queue.
    pub fn next_idx(&self) -> IdxTy {
        self.original_next_pos + self.new_entries.len()
    }

    /// Tries to add to the increment to the front of the queue.
    ///
    /// Returns if successful, fails if overflow or if there are insufficient
    /// entries to consume.
    pub fn add_front_incr(&mut self, incr: IncrTy) -> bool {
        // Incrementing by zero is a no-op, so should always succeed.
        if incr == 0 {
            return true;
        }

        // Check that there are entries available to consume (either in the
        // original queue or newly added).
        let next_idx = self.next_idx();
        if self.new_front_pos >= next_idx {
            // Front is already at or past the next position, so no entries to
            // consume.
            return false;
        }

        let incr_front = self.new_front_pos - self.original_front_pos;
        let new_front = (self.new_front_pos as u64) + (incr as u64);

        // So we don't overrun the back of the entries that'd be added..
        //
        // We allow new_front == next_idx (consuming all entries).
        if new_front > next_idx as u64 {
            return false;
        }

        let new_incr = (incr as u64) + (incr_front as u64);
        if new_incr >= HEAD_WORD_INCR_MASK as u64 {
            false
        } else {
            self.new_front_pos = new_front as IdxTy;
            true
        }
    }

    /// Appends an entry to the queue.
    ///
    /// Returns `true` if successful, `false` when [`Self::append_entry_with`]
    /// would refuse the entry.
    pub fn append_entry(&mut self, e: Q::Entry) -> bool {
        self.append_entry_with(move || e)
    }

    /// Invokes a closure to add an entry to the queue, iff there is space.
    ///
    /// This is useful when constructing the entry is expensive and you want to
    /// avoid the work if the queue is already full.
    ///
    /// Returns `true` if the entry was produced and added, `false` if the diff
    /// is full (at `TailLenTy::MAX` entries) or the queue's index type has no
    /// index left for the entry, the same bound `DaQueue::apply` enforces.
    pub fn append_entry_with(&mut self, f: impl FnOnce() -> Q::Entry) -> bool {
        if self.new_entries.len() >= TailLenTy::MAX as usize
            || !index_fits(
                self.original_next_pos as u64,
                self.new_entries.len() as u64 + 1,
            )
        {
            return false;
        }
        self.new_entries.push(f());
        true
    }
}

impl<Q: DaQueueTarget> DaBuilder<Q> for DaQueueBuilder<Q> {
    type Write = DaQueue<Q>;

    fn from_source(t: Q) -> Self {
        assert!(
            t.cur_front() <= t.cur_next(),
            "da/queue: source front {} is past its next index {}",
            t.cur_front(),
            t.cur_next()
        );
        Self {
            original_front_pos: t.cur_front() as IdxTy,
            new_front_pos: t.cur_front() as IdxTy,
            original_next_pos: t.cur_next() as IdxTy,
            new_entries: Vec::new(),
        }
    }

    fn into_write(self) -> Result<Self::Write, BuilderError> {
        // An entry appended and consumed in the same write stays in the tail
        // and the increment counts it: dropping it would renumber every later
        // entry, so an index taken from the builder's view would no longer
        // name the entry the applied queue holds there.
        //
        // `append_entry_with` kept every index within the type and
        // `from_source` checked the source, so the write applies.
        let tail = self.new_entries;
        let incr_front_idx = self.new_front_pos - self.original_front_pos;

        // Convert from IdxTy (usize) to IncrTy (u16)
        // This should always succeed because add_front_incr enforces
        // incr_front < HEAD_WORD_INCR_MASK
        let incr_front = incr_front_idx
            .try_into()
            .map_err(|_| BuilderError::OutOfBoundsValue)?;

        Ok(DaQueue { tail, incr_front })
    }
}

/// A view over a queue that combines the base state with pending changes from a
/// builder.
///
/// This provides a unified interface for reading from the effective queue state
/// (base + pending changes) while directing all writes to the builder.
pub struct QueueView<'a, Q: DaQueueTarget> {
    base: &'a Q,
    builder: &'a mut DaQueueBuilder<Q>,
}

impl<'a, Q: DaQueueTarget> QueueView<'a, Q> {
    /// Creates a new queue view over the given base and builder.
    pub fn new(base: &'a Q, builder: &'a mut DaQueueBuilder<Q>) -> Self {
        Self { base, builder }
    }

    /// Returns the effective front index of the queue (base + pending increments).
    pub fn front(&self) -> IdxTy {
        self.builder.new_front_pos
    }

    /// Returns the effective next index of the queue (base + pending appends).
    pub fn next(&self) -> IdxTy {
        self.builder.next_idx()
    }

    /// Returns the number of entries currently in the effective queue.
    pub fn len(&self) -> usize {
        let next = self.next();
        let front = self.front();
        next.saturating_sub(front)
    }

    /// Returns `true` if the effective queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gets the entry at the given index from the effective queue.
    ///
    /// Returns `None` if the index is before `front()` or at/past `next()`.
    pub fn get(&self, idx: IdxTy) -> Option<&Q::Entry> {
        // Check bounds
        if idx < self.front() || idx >= self.next() {
            return None;
        }

        // Check if it's in the new entries
        let original_next = self.builder.original_next_pos;
        if idx >= original_next {
            let new_idx = idx - original_next;
            return self.builder.new_entries.get(new_idx);
        }

        // It's in the base queue
        self.base.get(idx)
    }

    /// Appends an entry to the queue.
    ///
    /// Returns `true` if successful, `false` if the queue is full.
    pub fn append_entry(&mut self, e: Q::Entry) -> bool {
        self.builder.append_entry(e)
    }

    /// Appends an entry to the queue via a closure, only calling it if there's
    /// space.
    ///
    /// Returns `true` if successful, `false` if the queue is full.
    pub fn append_entry_with(&mut self, f: impl FnOnce() -> Q::Entry) -> bool {
        self.builder.append_entry_with(f)
    }

    /// Increments the front of the queue by the given amount.
    ///
    /// Returns `true` if successful, `false` if there are insufficient entries
    /// to consume or the increment would overflow.
    pub fn increment_front(&mut self, incr: IncrTy) -> bool {
        self.builder.add_front_incr(incr)
    }
}

#[cfg(test)]
mod tests {
    use proptest::{collection::vec, prelude::*};
    use strata_codec::BufDecoder;

    use super::*;
    use crate::CodecError;

    /// Helper to encode a CompoundMember to bytes.
    fn encode_set_cm_to_vec<T: CompoundMember>(v: &T) -> Result<Vec<u8>, CodecError> {
        let mut buf = Vec::new();
        v.encode_set(&mut buf)?;
        Ok(buf)
    }

    /// Helper to decode a CompoundMember from bytes.
    fn decode_set_cm_from_vec<T: CompoundMember>(buf: &[u8]) -> Result<T, CodecError> {
        let mut decoder = BufDecoder::new(buf);
        T::decode_set(&mut decoder)
    }

    /// Mock queue target for testing.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct MockQueue {
        front: IncrTy,
        next: IncrTy,
        entries: Vec<u32>,
    }

    impl DaQueueTarget for MockQueue {
        type Entry = u32;

        fn cur_front(&self) -> IncrTy {
            self.front
        }

        fn cur_next(&self) -> IncrTy {
            self.next
        }

        fn get(&self, idx: IdxTy) -> Option<&Self::Entry> {
            let front = self.front as IdxTy;
            let next = self.next as IdxTy;

            // Check bounds
            if idx < front || idx >= next {
                return None;
            }

            // Get from entries vec (indexed from front)
            let entries_idx = idx - front;
            self.entries.get(entries_idx)
        }

        fn insert_entries(&mut self, entries: &[Self::Entry]) {
            self.entries.extend_from_slice(entries);
            self.next += entries.len() as IncrTy;
        }

        fn increment_front(&mut self, incr: IncrTy) {
            // Consumed entries leave the queue, so `get` keeps indexing from
            // the front.
            self.entries.drain(..incr as usize);
            self.front += incr;
        }
    }

    fn base_queue_strategy() -> impl Strategy<Value = MockQueue> {
        (
            0u16..=(u16::MAX - 64),
            0usize..=32,
            vec(any::<u32>(), 0..=32),
        )
            .prop_flat_map(|(front, len, mut entries)| {
                entries.truncate(len);
                Just(MockQueue {
                    front,
                    next: front + (entries.len() as IncrTy),
                    entries,
                })
            })
    }

    fn tail_strategy() -> impl Strategy<Value = Vec<u32>> {
        vec(any::<u32>(), 0..=16)
    }

    fn valid_queue_write_strategy() -> impl Strategy<Value = (MockQueue, Vec<u32>, IncrTy)> {
        (base_queue_strategy(), tail_strategy(), 0usize..=48).prop_flat_map(
            |(base, tail, consume_hint)| {
                let available = base.entries.len() + tail.len();
                let consume = consume_hint.min(available) as IncrTy;
                Just((base, tail, consume))
            },
        )
    }

    #[test]
    fn test_encode_decode_head() {
        // Various checks for encoding/decoding the head word.
        let scenarios = vec![
            (false, 0),                   // empty, no increment
            (true, 0),                    // has entries, no increment
            (true, 100),                  // has entries and increment
            (false, 500),                 // no entries, with increment
            (false, HEAD_WORD_INCR_MASK), // max increment
            (true, HEAD_WORD_INCR_MASK),  // both flags at max
        ];

        for (has_entries, incr) in scenarios {
            let head = encode_head(has_entries, incr);
            let (decoded_has_entries, decoded_incr) = decode_head(head);
            assert_eq!(
                decoded_has_entries, has_entries,
                "test: has_entries mismatch for incr={}",
                incr
            );
            assert_eq!(decoded_incr, incr, "test: incr mismatch");
        }
    }

    #[test]
    #[should_panic(expected = "da/queue: tried to increment front by too much")]
    fn test_encode_head_overflow() {
        encode_head(false, HEAD_WORD_INCR_MASK + 1);
    }

    #[test]
    fn test_queue_encoding_scenarios() {
        // Test encoding/decoding with various queue configurations
        let scenarios = vec![
            // (tail, incr_front, expected_size_bytes)
            (vec![], 42, 2),                   // increment only: just head word
            (vec![10, 20, 30], 0, 2 + 2 + 12), // tail only: head + len + 3*u32
            (vec![100, 200, 300, 400], 15, 2 + 2 + 16), // both: head + len + 4*u32
            (vec![u32::MAX, 0], HEAD_WORD_INCR_MASK, 2 + 2 + 8), // boundary values
        ];

        for (tail, incr_front, expected_size) in scenarios {
            let queue = DaQueue::<MockQueue> {
                tail: tail.clone(),
                incr_front,
            };

            // Encode
            let buf = encode_set_cm_to_vec(&queue).expect("test :encode");
            assert_eq!(
                buf.len(),
                expected_size,
                "test: size mismatch for tail={:?}, incr={}",
                tail,
                incr_front
            );

            // Decode and verify
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&buf).expect("test: decode");
            assert_eq!(decoded.incr_front, incr_front);
            assert_eq!(decoded.tail, tail);
            assert_eq!(DaWrite::is_default(&decoded), DaWrite::is_default(&queue));
        }
    }

    #[test]
    fn test_queue_cm_refuses_default_and_its_other_spellings() {
        // The default write is a clear bit in the compound: neither side
        // spells it under a set bit, and a tail flag with no entries is the
        // flagless head spelled a second way.
        let default = <DaQueue<MockQueue> as Default>::default();
        assert!(matches!(
            encode_set_cm_to_vec(&default),
            Err(CodecError::InvalidVariant("queue"))
        ));
        let bare_zero_head = encode_head(false, 0).to_be_bytes();
        assert!(matches!(
            decode_set_cm_from_vec::<DaQueue<MockQueue>>(&bare_zero_head),
            Err(CodecError::InvalidVariant("queue"))
        ));
        let mut flagged_empty_tail = encode_head(true, 7).to_be_bytes().to_vec();
        flagged_empty_tail.extend_from_slice(&(0 as TailLenTy).to_be_bytes());
        assert!(matches!(
            decode_set_cm_from_vec::<DaQueue<MockQueue>>(&flagged_empty_tail),
            Err(CodecError::InvalidVariant("queue"))
        ));
        // The flagless head for the same increment is the one accepted form.
        let flagless = encode_head(false, 7).to_be_bytes();
        let decoded: DaQueue<MockQueue> =
            decode_set_cm_from_vec(&flagless).expect("test: canonical head decodes");
        assert_eq!(decoded.incr_front, 7);
        assert!(decoded.tail.is_empty());
    }

    #[test]
    fn test_queue_cm_enc_roundtrip() {
        // Comprehensive roundtrip test with various configurations
        let test_cases = vec![
            DaQueue::<MockQueue> {
                tail: Vec::new(),
                incr_front: 100,
            },
            DaQueue::<MockQueue> {
                tail: vec![1],
                incr_front: 0,
            },
            DaQueue::<MockQueue> {
                tail: vec![1, 2, 3, 4, 5],
                incr_front: 0,
            },
            DaQueue::<MockQueue> {
                tail: vec![42],
                incr_front: 10,
            },
            DaQueue::<MockQueue> {
                tail: vec![u32::MAX, 0, u32::MAX / 2],
                incr_front: HEAD_WORD_INCR_MASK,
            },
        ];

        for original in test_cases {
            let buf = encode_set_cm_to_vec(&original).expect("test: encode");
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&buf).expect("test: decode");

            assert_eq!(decoded.incr_front, original.incr_front);
            assert_eq!(decoded.tail, original.tail);
            assert_eq!(
                DaWrite::is_default(&decoded),
                DaWrite::is_default(&original)
            );
        }
    }

    #[test]
    #[should_panic(expected = "da/queue: tried to increment front by too much")]
    fn test_queue_cm_encode_above_mask_panics() {
        let queue = DaQueue::<MockQueue> {
            tail: vec![],
            incr_front: HEAD_WORD_INCR_MASK + 1,
        };
        let _ = encode_set_cm_to_vec(&queue);
    }

    #[test]
    fn test_queue_builder_append() {
        let source = MockQueue {
            front: 0,
            next: 3,
            entries: vec![1, 2, 3],
        };

        let mut builder = DaQueueBuilder::from_source(source);

        // Call in various different ways.
        let mut call_count = 0;
        assert!(builder.append_entry(10));
        assert!(builder.append_entry_with(|| {
            call_count += 1;
            20
        }));
        assert_eq!(call_count, 1);
        assert!(builder.append_entry_with(|| {
            call_count += 1;
            30
        }));
        assert_eq!(call_count, 2);
        assert_eq!(builder.new_entries, vec![10, 20, 30]);
    }

    #[test]
    fn test_queue_builder_append_bounds() {
        let source = MockQueue {
            front: 0,
            next: 0,
            entries: Vec::new(),
        };

        let mut builder = DaQueueBuilder::from_source(source);

        // Fill to capacity.
        for i in 0..TailLenTy::MAX {
            assert!(builder.append_entry(i as u32));
        }
        assert_eq!(builder.new_entries.len(), TailLenTy::MAX as usize);

        // Should reject new entries when at capacity.
        let mut closure_called = false;
        assert!(!builder.append_entry_with(|| {
            closure_called = true;
            999
        }));
        assert!(!closure_called);
    }

    #[test]
    fn test_queue_builder_increment_invalid() {
        // Allow zero increment.
        let mut builder = DaQueueBuilder::from_source(MockQueue {
            front: 0,
            next: 10,
            entries: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        });
        assert!(builder.add_front_incr(0));

        // Reject when empty.
        let mut builder = DaQueueBuilder::from_source(MockQueue {
            front: 0,
            next: 0,
            entries: Vec::new(),
        });
        assert!(!builder.add_front_incr(1));

        // Reject when front == next (non-zero).
        let mut builder = DaQueueBuilder::from_source(MockQueue {
            front: 5,
            next: 5,
            entries: Vec::new(),
        });
        assert!(!builder.add_front_incr(1));

        // Reject increment overflow.
        let mut builder = DaQueueBuilder::from_source(MockQueue {
            front: 0,
            next: u16::MAX,
            entries: Vec::new(),
        });
        assert!(!builder.add_front_incr(HEAD_WORD_INCR_MASK));
    }

    #[test]
    fn test_queue_builder_increment_bounds() {
        // Test increment boundary conditions
        let source = MockQueue {
            front: 0,
            next: 5,
            entries: vec![1, 2, 3, 4, 5],
        };
        let mut builder = DaQueueBuilder::from_source(source);

        // Can increment up to consuming all entries
        assert!(builder.add_front_incr(4));
        assert_eq!(builder.new_front_pos, 4);

        assert!(builder.add_front_incr(1));
        assert_eq!(builder.new_front_pos, 5); // front == next, empty

        // Cannot increment past all entries
        assert!(!builder.add_front_incr(1));
        assert_eq!(builder.new_front_pos, 5);
    }

    #[test]
    fn test_queue_builder_increment_base_only_into_write() {
        let source = MockQueue {
            front: 5,
            next: 7,
            entries: vec![1, 2],
        };
        let mut builder = DaQueueBuilder::from_source(source);
        assert!(builder.add_front_incr(1));

        let write = builder.into_write().expect("test: into_write");
        assert_eq!(write.incr_front, 1);
        assert!(write.tail.is_empty());
    }

    #[test]
    fn test_queue_builder_increment_with_new_entries() {
        let source = MockQueue {
            front: 0,
            next: 3,
            entries: vec![1, 2, 3],
        };
        let mut builder = DaQueueBuilder::from_source(source);

        // Consume all original entries.
        assert!(builder.add_front_incr(3));
        assert!(!builder.add_front_incr(1)); // no more entries

        // Add new entries.
        assert!(builder.append_entry(100));
        assert!(builder.append_entry(200));

        // Now can increment through new entries.
        assert!(builder.add_front_incr(1));
        assert_eq!(builder.new_front_pos, 4);
        assert!(builder.add_front_incr(1));
        assert_eq!(builder.new_front_pos, 5);

        // But not past them.
        assert!(!builder.add_front_incr(1));
    }

    #[test]
    fn test_queue_builder_append_incr_enc() {
        let source = MockQueue {
            front: 5,
            next: 5,
            entries: Vec::new(),
        };
        let mut builder = DaQueueBuilder::from_source(source);

        // Build changes.
        assert!(builder.append_entry(100));
        assert!(builder.append_entry(200));

        // Convert to write.
        let write = builder.into_write().expect("test: into_write");
        assert_eq!(write.incr_front, 0);
        assert_eq!(write.tail, vec![100, 200]);

        // Encode and decode.
        let buf = encode_set_cm_to_vec(&write).expect("test: encode");
        let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&buf).expect("test: decode");
        assert_eq!(decoded.tail, vec![100, 200]);
        assert_eq!(decoded.incr_front, 0);
    }

    #[test]
    fn test_queue_apply_operations() {
        // Various different application scenarios.
        let scenarios = vec![
            // (tail, incr_front, initial_queue, expected_queue)
            (
                vec![],
                0,
                MockQueue {
                    front: 0,
                    next: 0,
                    entries: vec![],
                },
                MockQueue {
                    front: 0,
                    next: 0,
                    entries: vec![],
                },
            ),
            (
                vec![],
                2,
                MockQueue {
                    front: 10,
                    next: 13,
                    entries: vec![1, 2, 3],
                },
                MockQueue {
                    front: 12,
                    next: 13,
                    entries: vec![3],
                },
            ),
            (
                vec![100, 200, 300],
                0,
                MockQueue {
                    front: 5,
                    next: 7,
                    entries: vec![1, 2],
                },
                MockQueue {
                    front: 5,
                    next: 10,
                    entries: vec![1, 2, 100, 200, 300],
                },
            ),
            (
                vec![7, 8, 9],
                2,
                MockQueue {
                    front: 0,
                    next: 5,
                    entries: vec![1, 2, 3, 4, 5],
                },
                MockQueue {
                    front: 2,
                    next: 8,
                    entries: vec![3, 4, 5, 7, 8, 9],
                },
            ),
        ];

        for (tail, incr_front, mut target, expected) in scenarios {
            let queue = DaQueue::<MockQueue> { tail, incr_front };
            queue.apply(&mut target, &()).expect("test: apply");

            assert_eq!(target.front, expected.front);
            assert_eq!(target.next, expected.next);
            assert_eq!(target.entries, expected.entries);
        }
    }

    #[test]
    fn test_queue_apply_rejects_front_overrun() {
        // A write off the wire is not bounded by the builder, so apply has to
        // bound the increment itself.
        let scenarios = vec![
            // (tail, incr_front, target)
            (
                vec![],
                1,
                MockQueue {
                    front: 0,
                    next: 0,
                    entries: vec![],
                },
            ),
            (
                vec![],
                4,
                MockQueue {
                    front: 10,
                    next: 13,
                    entries: vec![1, 2, 3],
                },
            ),
            // The tail counts, but only once.
            (
                vec![7, 8],
                6,
                MockQueue {
                    front: 0,
                    next: 3,
                    entries: vec![1, 2, 3],
                },
            ),
            (
                vec![],
                HEAD_WORD_INCR_MASK,
                MockQueue {
                    front: 5,
                    next: 7,
                    entries: vec![1, 2],
                },
            ),
        ];

        for (tail, incr_front, target) in scenarios {
            let write = DaQueue::<MockQueue> { tail, incr_front };
            let buf = encode_set_cm_to_vec(&write).expect("test: encode");
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&buf).expect("test: decode");

            let mut applied = target.clone();
            let err = decoded
                .apply(&mut applied, &())
                .expect_err("test: apply must reject the overrun");
            assert!(
                matches!(err, DaQueueError::FrontOverrun { .. }),
                "unexpected error {err:?}"
            );
            assert_eq!(
                applied, target,
                "a rejected write must not touch the target"
            );
        }

        // Consuming exactly everything, tail included, is fine.
        let write = DaQueue::<MockQueue> {
            tail: vec![7, 8],
            incr_front: 5,
        };
        let mut target = MockQueue {
            front: 0,
            next: 3,
            entries: vec![1, 2, 3],
        };
        write.apply(&mut target, &()).expect("test: apply");
        assert_eq!(target.front, 5);
        assert_eq!(target.next, 5);
        assert!(target.entries.is_empty(), "everything was consumed");
    }

    #[test]
    fn test_queue_apply_rejects_corrupt_target_and_index_overflow() {
        // A target whose front is already past its next index is not extended.
        let corrupt = MockQueue {
            front: 10,
            next: 5,
            entries: vec![],
        };
        let write = DaQueue::<MockQueue> {
            tail: vec![7, 8],
            incr_front: 0,
        };
        let mut applied = corrupt.clone();
        assert!(matches!(
            write.apply(&mut applied, &()),
            Err(DaQueueError::FrontPastNext { front: 10, next: 5 })
        ));
        assert_eq!(applied, corrupt);

        // A tail the index type cannot hold is refused before the counters wrap.
        let near_max = MockQueue {
            front: IncrTy::MAX - 5,
            next: IncrTy::MAX,
            entries: vec![1, 2, 3, 4, 5],
        };
        let write = DaQueue::<MockQueue> {
            tail: vec![9],
            incr_front: 5,
        };
        let mut applied = near_max.clone();
        assert!(matches!(
            write.apply(&mut applied, &()),
            Err(DaQueueError::IndexOverflow { added: 1, .. })
        ));
        assert_eq!(applied, near_max);

        // Without the tail the same increment empties the queue at the limit.
        let write = DaQueue::<MockQueue> {
            tail: vec![],
            incr_front: 5,
        };
        write.apply(&mut applied, &()).expect("test: apply");
        assert_eq!(applied.front, IncrTy::MAX);
        assert_eq!(applied.next, IncrTy::MAX);
        assert!(applied.entries.is_empty());
    }

    #[test]
    fn test_queue_builder_keeps_consumed_new_entries_in_write() {
        // An entry appended and consumed in the same write stays in the tail
        // and in the increment, so the surviving entry keeps the global index
        // the builder's view gave it.
        let source = MockQueue {
            front: 0,
            next: 3,
            entries: vec![1, 2, 3],
        };
        let mut builder = DaQueueBuilder::from_source(source.clone());
        assert!(builder.append_entry(100));
        assert!(builder.append_entry(200));
        assert!(builder.add_front_incr(4));
        let view_index_of_200 = {
            let view = QueueView::new(&source, &mut builder);
            assert_eq!(view.get(4), Some(&200));
            view.next() - 1
        };

        let write = builder.into_write().expect("test: into_write");
        assert_eq!(write.incr_front, 4);
        assert_eq!(write.tail, vec![100, 200]);

        let mut target = source;
        write.apply(&mut target, &()).expect("test: apply");
        assert_eq!(target.front, 4);
        assert_eq!(target.next, 5);
        assert_eq!(view_index_of_200, 4);
        assert_eq!(target.get(view_index_of_200), Some(&200));
        assert_eq!(target.get(3), None, "the consumed append is gone");
    }

    #[test]
    #[should_panic(expected = "source front 10 is past its next index 5")]
    fn test_queue_builder_rejects_a_corrupt_source() {
        let corrupt = MockQueue {
            front: 10,
            next: 5,
            entries: vec![],
        };
        let _ = DaQueueBuilder::from_source(corrupt);
    }

    #[test]
    fn test_queue_builder_refuses_appends_the_index_type_cannot_number() {
        // The builder stops where apply would refuse the write, so a write it
        // emits always applies.
        let full = MockQueue {
            front: IncrTy::MAX,
            next: IncrTy::MAX,
            entries: vec![],
        };
        let mut builder = DaQueueBuilder::from_source(full);
        assert!(!builder.append_entry(1));
        assert!(
            builder
                .into_write()
                .expect("test: into_write")
                .tail
                .is_empty()
        );

        let one_left = MockQueue {
            front: IncrTy::MAX - 1,
            next: IncrTy::MAX - 1,
            entries: vec![],
        };
        let mut builder = DaQueueBuilder::from_source(one_left.clone());
        assert!(builder.append_entry(1));
        assert!(!builder.append_entry(2));
        let write = builder.into_write().expect("test: into_write");
        let mut target = one_left;
        write.apply(&mut target, &()).expect("test: apply");
        assert_eq!(target.next, IncrTy::MAX);
        assert_eq!(target.entries, vec![1]);
    }

    #[test]
    fn test_queue_builder_full_roundtrip() {
        let mut queue = MockQueue {
            front: 5,
            next: 5,
            entries: Vec::new(),
        };

        // Build changes.
        let mut builder = DaQueueBuilder::from_source(queue.clone());
        assert!(builder.append_entry(100));
        assert!(builder.append_entry(200));

        // Convert to write.
        let write = builder.into_write().expect("test: into_write");

        // Encode and decode.
        let buf = encode_set_cm_to_vec(&write).expect("test: encode");
        let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&buf).expect("test: decode");

        // Apply to queue.
        decoded.apply(&mut queue, &()).expect("test: apply");

        // Verify final state.
        assert_eq!(queue.front, 5);
        assert_eq!(queue.next, 7);
        assert_eq!(queue.entries, vec![100, 200]);
    }

    #[test]
    fn test_queue_view() {
        let base = MockQueue {
            front: 0,
            next: 3,
            entries: vec![10, 20, 30],
        };

        let mut builder = DaQueueBuilder::from_source(base.clone());
        let mut view = QueueView::new(&base, &mut builder);

        // Initial state.
        assert_eq!(view.front(), 0);
        assert_eq!(view.next(), 3);
        assert_eq!(view.len(), 3);
        assert!(!view.is_empty());
        assert_eq!(view.get(0), Some(&10));
        assert_eq!(view.get(1), Some(&20));
        assert_eq!(view.get(2), Some(&30));
        assert_eq!(view.get(3), None);

        // Append entries.
        assert!(view.append_entry(40));
        let mut call_count = 0;
        assert!(view.append_entry_with(|| {
            call_count += 1;
            50
        }));
        assert_eq!(call_count, 1);
        assert_eq!(view.len(), 5);
        assert_eq!(view.get(3), Some(&40));
        assert_eq!(view.get(4), Some(&50));

        // Increment front.
        assert!(view.increment_front(1));
        assert_eq!(view.front(), 1);
        assert_eq!(view.len(), 4);
        assert_eq!(view.get(0), None); // consumed
        assert_eq!(view.get(1), Some(&20));
    }

    #[test]
    fn test_queue_view_boundary_cases() {
        let base = MockQueue {
            front: 0,
            next: 0,
            entries: Vec::new(),
        };
        let mut builder = DaQueueBuilder::from_source(base.clone());
        let view = QueueView::new(&base, &mut builder);

        assert_eq!(view.len(), 0);
        assert!(view.is_empty());
        assert_eq!(view.get(0), None);

        let base = MockQueue {
            front: 0,
            next: 3,
            entries: vec![10, 20, 30],
        };
        let mut builder = DaQueueBuilder::from_source(base.clone());
        let mut view = QueueView::new(&base, &mut builder);

        assert!(view.increment_front(3));
        assert_eq!(view.len(), 0);
        assert!(view.is_empty());
        for i in 0..5 {
            assert_eq!(view.get(i), None);
        }
    }

    #[test]
    fn test_queue_view_cross_boundary() {
        let base = MockQueue {
            front: 0,
            next: 2,
            entries: vec![10, 20],
        };

        let mut builder = DaQueueBuilder::from_source(base.clone());
        let mut view = QueueView::new(&base, &mut builder);

        // Add new entries.
        assert!(view.append_entry(30));
        assert!(view.append_entry(40));

        // Verify reading across boundary.
        assert_eq!(view.get(0), Some(&10)); // from base
        assert_eq!(view.get(1), Some(&20)); // from base
        assert_eq!(view.get(2), Some(&30)); // from new
        assert_eq!(view.get(3), Some(&40)); // from new

        // Increment past base into new entries.
        assert!(view.increment_front(3));
        assert_eq!(view.front(), 3);
        assert_eq!(view.len(), 1);
        assert_eq!(view.get(2), None);
        assert_eq!(view.get(3), Some(&40)); // only new entry visible
    }

    proptest! {
        #[test]
        fn proptest_queue_compound_member_roundtrip(
            tail in tail_strategy(),
            incr_front in 0u16..=HEAD_WORD_INCR_MASK,
        ) {
            let queue = DaQueue::<MockQueue> { tail, incr_front };
            // The default write goes out as a clear compound bit, not through `encode_set`.
            prop_assume!(!DaWrite::is_default(&queue));

            let encoded = encode_set_cm_to_vec(&queue).expect("test: encode queue");
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&encoded).expect("test: decode queue");

            prop_assert_eq!(decoded.tail, queue.tail);
            prop_assert_eq!(decoded.incr_front, queue.incr_front);
        }

        #[test]
        fn proptest_queue_builder_roundtrip_preserves_apply(
            (base, appended, consume_total) in valid_queue_write_strategy(),
        ) {
            let mut builder = DaQueueBuilder::from_source(base.clone());
            for entry in &appended {
                prop_assert!(builder.append_entry(*entry));
            }
            prop_assert_eq!(builder.add_front_incr(consume_total), (consume_total as usize) <= (base.entries.len() + appended.len()));

            let write = builder.into_write().expect("test: builder into_write");
            if <DaQueue<MockQueue> as DaWrite>::is_default(&write) {
                // The compound writes this one as a clear bit; the member refuses it.
                prop_assert!(matches!(
                    encode_set_cm_to_vec(&write),
                    Err(CodecError::InvalidVariant("queue"))
                ));
                return Ok(());
            }
            let encoded = encode_set_cm_to_vec(&write).expect("test: encode queue write");
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&encoded).expect("test: decode queue write");

            let mut applied_original = base.clone();
            let mut applied_decoded = base;
            write.apply(&mut applied_original, &()).expect("test: apply original write");
            decoded.apply(&mut applied_decoded, &()).expect("test: apply decoded write");

            prop_assert!(applied_original.front <= applied_original.next);
            prop_assert_eq!(applied_decoded, applied_original);
        }

        /// Any decoded write is accepted iff its increment fits the entries
        /// the queue holds once the tail is appended, and an accepted write
        /// never leaves the front past the next index.
        #[test]
        fn proptest_queue_apply_bounds_the_front_increment(
            base in base_queue_strategy(),
            tail in tail_strategy(),
            incr_front in 0u16..=HEAD_WORD_INCR_MASK,
        ) {
            let write = DaQueue::<MockQueue> { tail: tail.clone(), incr_front };
            prop_assume!(!DaWrite::is_default(&write));
            let encoded = encode_set_cm_to_vec(&write).expect("test: encode");
            let decoded: DaQueue<MockQueue> = decode_set_cm_from_vec(&encoded).expect("test: decode");

            let mut applied = base.clone();
            let result = decoded.apply(&mut applied, &());
            let fits = usize::from(incr_front) <= base.entries.len() + tail.len();
            prop_assert_eq!(result.is_ok(), fits);
            if fits {
                prop_assert!(applied.front <= applied.next);
                prop_assert_eq!(applied.front, base.front + incr_front);
                prop_assert_eq!(applied.next, base.next + tail.len() as IncrTy);
            } else {
                prop_assert_eq!(applied, base);
            }
        }

        #[test]
        fn proptest_queue_builder_into_write_keeps_consumed_new_entries(
            base in base_queue_strategy(),
            appended in tail_strategy(),
            consume_hint in 0usize..=48,
        ) {
            let base_len = base.entries.len();
            let consumed_total = consume_hint.min(base_len + appended.len());

            let mut builder = DaQueueBuilder::from_source(base.clone());
            for entry in &appended {
                prop_assert!(builder.append_entry(*entry));
            }
            prop_assert!(builder.add_front_incr(consumed_total as IncrTy));
            let (view_front, view_next, view_entries) = {
                let view = QueueView::new(&base, &mut builder);
                let entries: Vec<_> = (view.front()..view.next())
                    .map(|i| view.get(i).copied())
                    .collect();
                (view.front(), view.next(), entries)
            };

            let write = builder.into_write().expect("test: builder into_write");

            prop_assert_eq!(write.incr_front, consumed_total as IncrTy);
            prop_assert_eq!(&write.tail, &appended);

            // Every index the view handed out names the same entry in the
            // applied queue, and the queue holds nothing else.
            let mut applied = base.clone();
            write.apply(&mut applied, &()).expect("test: apply");
            prop_assert_eq!(applied.front as usize, view_front);
            prop_assert_eq!(applied.next as usize, view_next);
            let applied_entries: Vec<_> = (view_front..view_next)
                .map(|i| applied.get(i).copied())
                .collect();
            prop_assert_eq!(applied_entries, view_entries);
            prop_assert_eq!(applied.entries.len(), view_next - view_front);
        }
    }
}

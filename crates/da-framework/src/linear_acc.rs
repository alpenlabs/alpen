//! Linear accumulator DA pattern.
//!
//! This is for types that we always insert into a "back end of", like a MMR,
//! a DBLMA, or even a simple hash chain.

use std::default::Default;

use crate::{
    BuilderError, Codec, CodecError, CodecResult, CompoundMember, DaBuilder, DaWrite, Decoder,
    Encoder,
};

/// Describes an accumulator we can insert entries into the back of.
pub trait LinearAccumulator {
    /// Insert count type.
    ///
    /// This should just be an integer value.
    type InsertCnt: Copy + Eq + Ord + Codec + From<usize> + Into<usize>;

    /// Entry type.
    type EntryData: Clone + Codec;

    /// The maximum number of entries we can insert at once.
    ///
    /// This should be a `::MAX` integer value.
    const MAX_INSERT: Self::InsertCnt;

    /// Inserts an entry into the growing end of the accumulator.
    fn insert(&mut self, entry: &Self::EntryData);
}

/// Describes a write to a linear accumulator.
#[derive(Clone)]
pub struct DaLinacc<A: LinearAccumulator> {
    new_entries: Vec<A::EntryData>,
}

impl<A: LinearAccumulator> DaLinacc<A> {
    /// Constructs a new instance.
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Gets the slice of new entries.
    pub fn new_entries(&self) -> &[A::EntryData] {
        &self.new_entries
    }

    /// Returns if the write is full and cannot accept new entries
    pub fn is_write_full(&self) -> bool {
        let val = <A::InsertCnt as From<usize>>::from(self.new_entries.len());
        val >= A::MAX_INSERT
    }

    /// Appends a new entry that we'll insert into the back.
    ///
    /// Returns if the append was accepted.  This only accepts if
    /// `is_write_full` returns false.
    pub fn append_entry(&mut self, e: A::EntryData) -> bool {
        if !self.is_write_full() {
            self.new_entries.push(e);
            true
        } else {
            false
        }
    }
}

impl<A: LinearAccumulator> Default for DaLinacc<A> {
    fn default() -> Self {
        Self {
            new_entries: Vec::new(),
        }
    }
}

impl<A: LinearAccumulator> DaWrite for DaLinacc<A> {
    type Target = A;

    fn is_default(&self) -> bool {
        self.new_entries.is_empty()
    }

    fn apply(&self, target: &mut Self::Target) {
        for e in &self.new_entries {
            target.insert(e);
        }
    }
}

impl<A: LinearAccumulator> CompoundMember for DaLinacc<A> {
    fn default() -> Self {
        <Self as Default>::default()
    }

    fn is_default(&self) -> bool {
        self.new_entries.is_empty()
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        // Decode the counter and bounds check it.
        let cnt = <A::InsertCnt as Codec>::decode(dec)?;

        if cnt > A::MAX_INSERT {
            return Err(CodecError::OversizeContainer);
        }

        let cnt: usize = cnt.into();

        // Decode each entry.
        let mut new_entries = Vec::new();
        for _ in 0..cnt {
            let e = <A::EntryData as Codec>::decode(dec)?;
            new_entries.push(e);
        }

        Ok(Self { new_entries })
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        // Encode the counter.
        let cnt: A::InsertCnt = self.new_entries.len().into();
        cnt.encode(enc)?;

        // Encode each entry.
        for e in &self.new_entries {
            e.encode(enc)?;
        }

        Ok(())
    }
}

/// Builder for [`DaLinaccc`].
///
/// Arguably this is redundant and we can use the DA type directly.
pub struct DaLinaccBuilder<A: LinearAccumulator> {
    new_entries: Vec<A::EntryData>,
}

impl<A: LinearAccumulator> DaLinaccBuilder<A> {
    pub fn append_entry(&mut self, v: A::EntryData) {
        self.new_entries.push(v);
    }
}

impl<A: LinearAccumulator> DaBuilder<A> for DaLinaccBuilder<A> {
    type Write = DaLinacc<A>;

    fn from_source(_t: A) -> Self {
        Self {
            new_entries: Vec::new(),
        }
    }

    fn into_write(self) -> Result<Self::Write, BuilderError> {
        Ok(DaLinacc {
            new_entries: self.new_entries,
        })
    }
}

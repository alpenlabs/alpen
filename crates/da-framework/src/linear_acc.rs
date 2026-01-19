//! Linear accumulator DA pattern.
//!
//! This is for types that we always insert into a "back end of", like a MMR,
//! a DBLMA, or even a simple hash chain.

use std::default::Default;

use crate::{Codec, CodecError, CodecResult, CompoundMember, DaWrite, Decoder, Encoder};

/// Describes an accumulator we can insert entries into the back of.
pub trait LinearAccumulator {
    /// Insert count type.
    ///
    /// This should just be an integer value.
    type InsertCnt: Copy + Eq + Ord + Codec + TryFrom<usize> + TryInto<usize>;

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
        let Ok(val) = <A::InsertCnt as TryFrom<usize>>::try_from(self.new_entries.len()) else {
            // If we get here then it means we somehow exceeded the limit.
            panic!("da/linacc: buffer overfilled");
        };
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

    type Context = ();

    fn is_default(&self) -> bool {
        self.new_entries.is_empty()
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), crate::DaError> {
        for e in &self.new_entries {
            target.insert(e);
        }
        Ok(())
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
            return Err(CodecError::OverflowContainer);
        }

        let cnt: usize = cnt.try_into().map_err(|_| CodecError::OverflowContainer)?;

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
        let cnt: A::InsertCnt = self
            .new_entries
            .len()
            .try_into()
            .map_err(|_| CodecError::OverflowContainer)?;
        cnt.encode(enc)?;

        // Encode each entry.
        for e in &self.new_entries {
            e.encode(enc)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::BufDecoder;
    use strata_merkle::{CompactMmr64, Mmr, MmrState, Sha256Hasher, hasher::MerkleHasher};

    use super::*;
    use crate::{CompoundMember, DaWrite};

    /// Wrapper for CompactMmr64 to implement LinearAccumulator
    struct TestMmr(CompactMmr64<[u8; 32]>);

    impl TestMmr {
        fn new() -> Self {
            Self(CompactMmr64::new(14))
        }
    }

    impl LinearAccumulator for TestMmr {
        type InsertCnt = u64;
        type EntryData = [u8; 128];
        const MAX_INSERT: Self::InsertCnt = u64::MAX;

        fn insert(&mut self, entry: &Self::EntryData) {
            let hash = Sha256Hasher::hash_leaf(entry);
            Mmr::<Sha256Hasher>::add_leaf(&mut self.0, hash).expect("test: insert should succeed");
        }
    }

    #[test]
    fn test_mmr_linear_accumulator_round_trip() {
        // Create initial MMR
        let mut mmr = TestMmr::new();

        // Create some test entries
        let entry1 = [1u8; 128];
        let entry2 = [2u8; 128];
        let entry3 = [3u8; 128];

        // Build a diff with multiple entries
        let mut diff = DaLinacc::<TestMmr>::new();
        assert!(diff.append_entry(entry1));
        assert!(diff.append_entry(entry2));
        assert!(diff.append_entry(entry3));

        // Capture initial MMR state
        let initial_entries = mmr.0.num_entries();

        // Apply the diff to get the "after" state
        DaWrite::apply(&diff, &mut mmr, &()).expect("test: apply should succeed");
        let after_entries = mmr.0.num_entries();

        // Verify entries were added
        assert_eq!(after_entries, initial_entries + 3);

        // Encode the diff using CompoundMember
        let mut encoder = Vec::new();
        CompoundMember::encode_set(&diff, &mut encoder).expect("test: encode should succeed");

        // Decode the diff using CompoundMember
        let mut decoder = BufDecoder::new(&encoder);
        let decoded: DaLinacc<TestMmr> =
            CompoundMember::decode_set(&mut decoder).expect("test: decode should succeed");

        // Verify the decoded diff matches the original
        assert_eq!(decoded.new_entries.len(), diff.new_entries.len());
        assert_eq!(decoded.new_entries[0], entry1);
        assert_eq!(decoded.new_entries[1], entry2);
        assert_eq!(decoded.new_entries[2], entry3);

        // Apply the decoded diff to a fresh MMR
        let mut mmr2 = TestMmr::new();
        DaWrite::apply(&decoded, &mut mmr2, &()).expect("test: apply should succeed");

        // Verify that both MMRs have the same final state
        assert_eq!(mmr2.0.num_entries(), mmr.0.num_entries());
        assert_eq!(mmr2.0.num_entries(), 3);

        // Verify that the peaks are identical
        let peaks1: Vec<_> = mmr.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        let peaks2: Vec<_> = mmr2.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        assert_eq!(peaks1, peaks2, "test: MMR peaks should be identical");
    }

    #[test]
    fn test_mmr_empty_diff() {
        let mut mmr = TestMmr::new();
        let diff = DaLinacc::<TestMmr>::new();

        // Empty diff should be default
        assert!(DaWrite::is_default(&diff));

        // Encode and decode empty diff
        let mut encoder = Vec::new();
        CompoundMember::encode_set(&diff, &mut encoder).expect("test: encode should succeed");
        let mut decoder = BufDecoder::new(&encoder);
        let decoded: DaLinacc<TestMmr> =
            CompoundMember::decode_set(&mut decoder).expect("test: decode should succeed");

        // Apply decoded diff - should be a no-op
        let initial_entries = mmr.0.num_entries();
        let initial_peaks: Vec<_> = mmr.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        DaWrite::apply(&decoded, &mut mmr, &()).expect("test: apply should succeed");
        assert_eq!(mmr.0.num_entries(), initial_entries);

        // Verify peaks unchanged
        let final_peaks: Vec<_> = mmr.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        assert_eq!(
            initial_peaks, final_peaks,
            "test: empty diff should not change MMR peaks"
        );
    }

    #[test]
    fn test_mmr_single_entry() {
        let mut mmr = TestMmr::new();
        let entry = [42u8; 128];

        let mut diff = DaLinacc::<TestMmr>::new();
        assert!(diff.append_entry(entry));

        // Encode/decode round trip
        let mut encoder = Vec::new();
        CompoundMember::encode_set(&diff, &mut encoder).expect("test: encode should succeed");
        let mut decoder = BufDecoder::new(&encoder);
        let decoded: DaLinacc<TestMmr> =
            CompoundMember::decode_set(&mut decoder).expect("test: decode should succeed");

        // Apply the decoded diff
        DaWrite::apply(&decoded, &mut mmr, &()).expect("test: apply should succeed");
        assert_eq!(mmr.0.num_entries(), 1);

        // Apply original diff to a fresh MMR and compare peaks
        let mut mmr2 = TestMmr::new();
        DaWrite::apply(&diff, &mut mmr2, &()).expect("test: apply should succeed");

        let peaks1: Vec<_> = mmr.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        let peaks2: Vec<_> = mmr2.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        assert_eq!(
            peaks1, peaks2,
            "test: MMR peaks should be identical after applying decoded diff"
        );
    }

    #[test]
    fn test_mmr_sequential_diffs() {
        // Test applying multiple diffs sequentially
        let mut mmr = TestMmr::new();

        // First diff
        let mut diff1 = DaLinacc::<TestMmr>::new();
        diff1.append_entry([1u8; 128]);
        diff1.append_entry([2u8; 128]);

        // Second diff
        let mut diff2 = DaLinacc::<TestMmr>::new();
        diff2.append_entry([3u8; 128]);
        diff2.append_entry([4u8; 128]);

        // Apply first diff
        DaWrite::apply(&diff1, &mut mmr, &()).expect("test: apply should succeed");
        assert_eq!(mmr.0.num_entries(), 2);

        // Apply second diff
        DaWrite::apply(&diff2, &mut mmr, &()).expect("test: apply should succeed");
        assert_eq!(mmr.0.num_entries(), 4);

        // Now verify the same result with encoded/decoded diffs
        let mut mmr2 = TestMmr::new();

        // Encode/decode/apply first diff
        let mut encoder1 = Vec::new();
        CompoundMember::encode_set(&diff1, &mut encoder1).expect("test: encode should succeed");
        let mut decoder1 = BufDecoder::new(&encoder1);
        let decoded1: DaLinacc<TestMmr> =
            CompoundMember::decode_set(&mut decoder1).expect("test: decode should succeed");
        DaWrite::apply(&decoded1, &mut mmr2, &()).expect("test: apply should succeed");

        // Encode/decode/apply second diff
        let mut encoder2 = Vec::new();
        CompoundMember::encode_set(&diff2, &mut encoder2).expect("test: encode should succeed");
        let mut decoder2 = BufDecoder::new(&encoder2);
        let decoded2: DaLinacc<TestMmr> =
            CompoundMember::decode_set(&mut decoder2).expect("test: decode should succeed");
        DaWrite::apply(&decoded2, &mut mmr2, &()).expect("test: apply should succeed");

        // Both MMRs should have the same final state
        assert_eq!(mmr2.0.num_entries(), mmr.0.num_entries());
        assert_eq!(mmr2.0.num_entries(), 4);

        // Verify that the peaks are identical
        let peaks1: Vec<_> = mmr.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        let peaks2: Vec<_> = mmr2.0.iter_peaks().map(|(h, p)| (h, *p)).collect();
        assert_eq!(
            peaks1, peaks2,
            "test: MMR peaks should be identical after sequential diffs"
        );
    }
}

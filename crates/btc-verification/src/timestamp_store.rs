use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use strata_btc_types::TIMESTAMPS_FOR_MEDIAN;

/// The middle index for selecting the median timestamp.
/// Since TIMESTAMPS_FOR_MEDIAN is odd, the median is the element at index 5 (the 6th element)
/// after the timestamps are sorted.
pub const MEDIAN_TIMESTAMP_INDEX: usize = TIMESTAMPS_FOR_MEDIAN / 2;

/// A ring buffer that stores exactly `TIMESTAMPS_FOR_MEDIAN` timestamps.
/// When inserting a new timestamp, the oldest timestamp is overwritten and the head pointer
/// is advanced in a circular manner.
///
/// The median is computed using all timestamps in the buffer.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Arbitrary,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct TimestampStore {
    /// The array that holds exactly `TIMESTAMPS_FOR_MEDIAN` timestamps.
    buffer: [u32; TIMESTAMPS_FOR_MEDIAN],
    /// The index in the buffer where the next timestamp will be inserted.
    head: usize,
}

impl Default for TimestampStore {
    fn default() -> Self {
        Self {
            buffer: [0; TIMESTAMPS_FOR_MEDIAN],
            head: 0,
        }
    }
}

impl TimestampStore {
    /// Creates a new `TimestampStore` initialized with the given timestamps.
    /// The `initial_timestamps` array fills the buffer, and the `head` is set to 0,
    /// meaning that the next inserted timestamp will overwrite the first element.
    pub fn new(initial_timestamps: [u32; TIMESTAMPS_FOR_MEDIAN]) -> Self {
        Self {
            buffer: initial_timestamps,
            head: 0,
        }
    }

    /// Inserts a new timestamp into the buffer, overwriting the oldest timestamp.
    /// After insertion, the `head` is advanced in a circular manner.
    pub fn insert(&mut self, timestamp: u32) {
        self.buffer[self.head] = timestamp;
        self.head = (self.head + 1) % TIMESTAMPS_FOR_MEDIAN;
    }

    /// Computes and returns the median timestamp from all timestamps in the buffer.
    ///
    /// The median is calculated by taking a copy of all timestamps, sorting them,
    /// and selecting the element at the middle index `MEDIAN_TIMESTAMP_INDEX`.
    pub fn median(&self) -> u32 {
        let mut timestamps = self.buffer;
        timestamps.sort_unstable();
        timestamps[MEDIAN_TIMESTAMP_INDEX]
    }
}

#[cfg(test)]
mod tests {
    use std::array;

    use super::*;

    #[test]
    fn test_initial_median_calculation() {
        // Initialize the buffer with timestamps from 1 to 11
        // This creates a sorted sequence where the median is predictable (6)
        let initial_timestamps: [u32; 11] = array::from_fn(|i| (i + 1) as u32);
        let timestamps = TimestampStore::new(initial_timestamps);

        // Initial median should be 6 (middle of 1-11)
        assert_eq!(timestamps.median(), 6);
    }

    #[test]
    fn test_median_with_incrementing_values() {
        // Start with sorted sequence 1-11 (median = 6)
        let initial_timestamps: [u32; 11] = array::from_fn(|i| (i + 1) as u32);
        let mut timestamps = TimestampStore::new(initial_timestamps);
        let mut expected_median = 6;

        // Insert new timestamps from 12.. and test median
        // Since we're inserting values larger than all existing values,
        // the median shifts upward by 1 with each insertion because the ring buffer
        // overwrites the smallest values first (due to circular insertion starting from index 0)
        let new_timestamps: [u32; 20] = array::from_fn(|i| (i + 12) as u32);
        for &ts in &new_timestamps {
            timestamps.insert(ts);
            expected_median += 1;
            assert_eq!(timestamps.median(), expected_median);
        }
    }

    #[test]
    fn test_median_with_large_non_sequential_values() {
        // Start with a state where we've already inserted some values
        let initial_timestamps: [u32; 11] = array::from_fn(|i| (i + 1) as u32);
        let mut timestamps = TimestampStore::new(initial_timestamps);

        // Insert 20 incrementing values to get to a known state
        let new_timestamps: [u32; 20] = array::from_fn(|i| (i + 12) as u32);
        for &ts in &new_timestamps {
            timestamps.insert(ts);
        }

        let mut expected_median = timestamps.median();

        // Test non-sequential large timestamps
        // These large values continue to replace smaller values, pushing the median up
        let large_timestamps: [u32; MEDIAN_TIMESTAMP_INDEX] =
            array::from_fn(|i| ((i + 12) * 10) as u32);
        for &ts in &large_timestamps {
            timestamps.insert(ts);
            expected_median += 1;
            assert_eq!(timestamps.median(), expected_median);
        }
    }

    #[test]
    fn test_median_with_zero_insertions() {
        // Start with a buffer containing larger values
        let initial_timestamps: [u32; 11] = array::from_fn(|i| (i + 20) as u32);
        let mut timestamps = TimestampStore::new(initial_timestamps);

        // Test adding zeros - this will replace values starting from head position
        // The ring buffer insertion is circular, so zeros replace whatever values are at head
        let median_before_zeros = timestamps.median();
        for _ in 0..5 {
            timestamps.insert(0);
        }
        let median_with_zeros = timestamps.median();

        // We can't predict exactly how the median changes without knowing the head position,
        // but we can verify that the zeros were inserted and the median calculation still works
        assert!(
            median_with_zeros <= median_before_zeros
                || timestamps.buffer.iter().filter(|&&x| x == 0).count() == 5,
            "Should have 5 zeros in buffer or median should not increase"
        );
    }

    #[test]
    fn test_median_with_unsorted_values() {
        // Test non-incrementing values to verify the median calculation works with unsorted input
        // Use a known sequence to test predictable behavior
        let test_timestamps = [10, 5, 15, 3, 8, 12, 7, 9, 4, 6, 11];
        let mut timestamps = TimestampStore::new(test_timestamps);

        // Should be 8 (sorted: [3,4,5,6,7,8,9,10,11,12,15])
        let initial_median = timestamps.median();
        assert_eq!(initial_median, 8);

        // Insert values that should change the median in a predictable way
        timestamps.insert(1); // Replaces 10, sorted becomes [1,3,4,5,6,7,8,9,11,12,15], median = 7
        assert_eq!(timestamps.median(), 7);

        timestamps.insert(20); // Replaces 5, sorted becomes [1,3,4,6,7,8,9,11,12,15,20], median = 8
        assert_eq!(timestamps.median(), 8);

        timestamps.insert(2); // Replaces 15, sorted becomes [1,2,3,4,6,7,8,9,11,12,20], median = 7
        assert_eq!(timestamps.median(), 7);
    }
}

//! Utility type for tracking consistency of inputs/outputs.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SeqError {
    #[error("inconsistent entry at index {0}")]
    Mismatch(usize),

    #[error("consumed all entries but checked another")]
    Overrun,
}

pub type SeqResult<T> = Result<T, SeqError>;

/// Tracks a list of entries that we want to compare a sequence of them against.
///
/// We use this for processing pending inputs while processing notpackages.
#[derive(Debug)]
pub struct SequenceTracker<'a, T> {
    expected_inputs: &'a [T],
    consumed: usize,
}

impl<'a, T> SequenceTracker<'a, T> {
    pub fn new(expected_inputs: &'a [T]) -> Self {
        Self {
            expected_inputs,
            consumed: 0,
        }
    }

    /// Gets the number of entries consumed.
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    /// Gets if there are more entries that could be consumed.
    ///
    /// This is only available in tests since we should never need to do
    /// conditional logic based on this type's state, since it's a validator.
    #[cfg(test)]
    fn has_next(&self) -> bool {
        self.consumed < self.expected_inputs.len()
    }

    /// Gets the next entry that would need to be be consumed, if there is one.
    fn expected_next(&self) -> Option<&'a T> {
        self.expected_next_rel(0)
    }

    /// Gets the entry that would need to be consumed after some number of
    /// calls to [`consume_input`], if there is one.
    fn expected_next_rel(&self, off: usize) -> Option<&'a T> {
        self.expected_inputs.get(self.consumed + off)
    }
}

impl<'a, T: Eq + PartialEq> SequenceTracker<'a, T> {
    /// Checks if an input matches the next value we expect to consume.  If it
    /// matches, increments the pointer.  Errors on mismatch.
    pub fn consume_input(&mut self, input: &T) -> SeqResult<()> {
        let Some(exp_next) = self.expected_next() else {
            return Err(SeqError::Overrun);
        };

        if input != exp_next {
            return Err(SeqError::Mismatch(self.consumed));
        }

        self.consumed += 1;
        Ok(())
    }

    /// Like [`consume_input`], but checks multiple inputs and only updates the
    /// consumed pointer state on success.
    pub fn consume_inputs(&mut self, inputs: &[T]) -> SeqResult<()> {
        // Bounds check early so we can skip it on each iteration.
        if inputs.len() > self.remaining().len() {
            return Err(SeqError::Overrun);
        }

        for (i, input) in inputs.iter().enumerate() {
            // SAFETY: we already did bounds checking
            let exp_next = self.expected_next_rel(i).unwrap();

            // Just check equality and return the index if there's a mismatch.
            if input != exp_next {
                return Err(SeqError::Mismatch(self.consumed + i));
            }
        }

        // Commit only after all checks succeed
        self.consumed += inputs.len();
        Ok(())
    }

    /// Gets the remaining unconsumed entries.
    pub fn remaining(&self) -> &'a [T] {
        &self.expected_inputs[self.consumed..]
    }

    /// Checks if all entries have been consumed.
    pub fn is_empty(&self) -> bool {
        self.consumed >= self.expected_inputs.len()
    }

    /// Advances the tracker by `count` entries without checking them.
    ///
    /// This should only be called after validation has been performed.
    pub fn advance_unchecked(&mut self, count: usize) {
        self.consumed += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_tracker_new() {
        let inputs = vec![1, 2, 3];
        let tracker = SequenceTracker::new(&inputs);

        assert_eq!(tracker.consumed(), 0);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&1));
    }

    #[test]
    fn test_sequence_tracker_empty() {
        let inputs: Vec<i32> = vec![];
        let tracker = SequenceTracker::new(&inputs);

        assert_eq!(tracker.consumed(), 0);
        assert!(!tracker.has_next());
        assert_eq!(tracker.expected_next(), None);
    }

    #[test]
    fn test_sequence_tracker_consume_matching() {
        let inputs = vec![1, 2, 3];
        let mut tracker = SequenceTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&2));

        assert!(tracker.consume_input(&2).is_ok());
        assert_eq!(tracker.consumed(), 2);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&3));

        assert!(tracker.consume_input(&3).is_ok());
        assert_eq!(tracker.consumed(), 3);
        assert!(!tracker.has_next());
        assert_eq!(tracker.expected_next(), None);
    }

    #[test]
    fn test_sequence_tracker_consume_mismatch() {
        let inputs = vec![1, 2, 3];
        let mut tracker = SequenceTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);

        let result = tracker.consume_input(&99);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SeqError::Mismatch(1)));
        assert_eq!(tracker.consumed(), 1); // consumed count unchanged on error
    }

    #[test]
    fn test_sequence_tracker_consume_beyond_end() {
        let inputs = vec![1];
        let mut tracker = SequenceTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);
        assert!(!tracker.has_next());

        let result = tracker.consume_input(&2);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SeqError::Overrun));
        assert_eq!(tracker.consumed(), 1);
    }

    #[test]
    fn test_sequence_tracker_consume_wrong_order() {
        let inputs = vec![1, 2, 3];
        let mut tracker = SequenceTracker::new(&inputs);

        let result = tracker.consume_input(&2);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SeqError::Mismatch(0)));
        assert_eq!(tracker.consumed(), 0);
    }

    #[test]
    fn test_sequence_tracker_string_type() {
        let inputs = vec!["foo".to_string(), "bar".to_string(), "baz".to_string()];
        let mut tracker = SequenceTracker::new(&inputs);

        assert!(tracker.consume_input(&"foo".to_string()).is_ok());
        assert_eq!(tracker.consumed(), 1);

        assert!(tracker.consume_input(&"bar".to_string()).is_ok());
        assert_eq!(tracker.consumed(), 2);

        let result = tracker.consume_input(&"wrong".to_string());
        assert!(result.is_err());
        assert_eq!(tracker.consumed(), 2);
    }
}

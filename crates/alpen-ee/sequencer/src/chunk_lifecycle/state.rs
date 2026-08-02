//! Chunk proof lifecycle state.

/// In-memory fairness state for chunk proof work.
///
/// Chunk work discovery itself is storage-driven: sealed and proof-pending chunks are queried from
/// storage by status each tick. This state only remembers where the next status page should begin,
/// so a bad chunk near index 0 cannot starve later chunks.
#[derive(Debug, Default)]
pub(super) struct ChunkLifecycleState {
    sealed_poll_idx: u64,
    pending_poll_idx: u64,
}

/// Positions a work cursor for the next tick given the page just processed.
///
/// A page shorter than the query limit means the queue was read to its end, so the cursor resets to
/// the bottom of the index space and the next tick rescans from there. That is what keeps a chunk
/// left behind by a failed submission from being starved: without it, a steady stream of newly
/// sealed chunks keeps every page non-empty and the cursor never returns to the straggler. The cost
/// is re-reading at most one page of still-active rows per tick, and both read paths re-verify the
/// chunk's status before acting on it. A full page means there is more work above, so the cursor
/// advances past the last row instead.
fn next_poll_idx(page_last_idx: Option<u64>, page_was_full: bool) -> u64 {
    match page_last_idx.filter(|_| page_was_full) {
        Some(idx) => idx.saturating_add(1),
        None => 0,
    }
}

impl ChunkLifecycleState {
    /// The first chunk index to consider when submitting sealed chunks.
    pub(super) fn sealed_poll_idx(&self) -> u64 {
        self.sealed_poll_idx
    }

    /// Position the sealed-work cursor after processing a page of chunks.
    ///
    /// See [`next_poll_idx`] for the rule.
    pub(super) fn record_sealed_page(&mut self, page_last_idx: Option<u64>, page_was_full: bool) {
        self.sealed_poll_idx = next_poll_idx(page_last_idx, page_was_full);
    }

    /// The first chunk index to consider when polling pending proof tasks.
    pub(super) fn pending_poll_idx(&self) -> u64 {
        self.pending_poll_idx
    }

    /// Position the pending-work cursor after polling a page of chunks.
    ///
    /// See [`next_poll_idx`] for the rule.
    pub(super) fn record_pending_page(&mut self, page_last_idx: Option<u64>, page_was_full: bool) {
        self.pending_poll_idx = next_poll_idx(page_last_idx, page_was_full);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full page moves the cursor past its last row, and the two cursors move independently.
    #[test]
    fn full_work_pages_advance_cursors_independently() {
        let mut state = ChunkLifecycleState::default();
        assert_eq!(state.sealed_poll_idx(), 0);
        assert_eq!(state.pending_poll_idx(), 0);

        state.record_sealed_page(Some(3), true);
        assert_eq!(state.sealed_poll_idx(), 4);
        assert_eq!(state.pending_poll_idx(), 0);

        state.record_pending_page(Some(7), true);
        assert_eq!(state.sealed_poll_idx(), 4);
        assert_eq!(state.pending_poll_idx(), 8);
    }

    /// A short or empty page means the end of the queue was reached, so the cursor rewinds to 0 and
    /// the next tick rescans lower indexes.
    #[test]
    fn short_work_pages_reset_cursors_to_zero() {
        let mut state = ChunkLifecycleState::default();
        state.record_sealed_page(Some(3), true);
        state.record_pending_page(Some(7), true);

        state.record_sealed_page(Some(9), false);
        state.record_pending_page(None, false);
        assert_eq!(state.sealed_poll_idx(), 0);
        assert_eq!(state.pending_poll_idx(), 0);
    }
}

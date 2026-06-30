//! Chunk proof lifecycle state.

/// In-memory fairness state for chunk proof work.
///
/// Chunk work discovery itself is storage-driven: sealed and proof-pending chunks are queried from
/// storage by status each tick. This state only remembers where the next status page should begin,
/// so a bad chunk near index 0 cannot starve later chunks.
#[derive(Debug, Default)]
pub(super) struct ChunkLifecycleState {
    pending_poll_idx: u64,
}

impl ChunkLifecycleState {
    /// The first chunk index to consider when polling pending proof tasks.
    pub(super) fn pending_poll_idx(&self) -> u64 {
        self.pending_poll_idx
    }

    /// Advance the pending poll cursor after polling a page of chunks.
    pub(super) fn advance_pending_poll_idx(&mut self, last_polled_idx: Option<u64>) {
        if let Some(idx) = last_polled_idx {
            self.pending_poll_idx = idx.saturating_add(1);
        }
    }

    /// Wrap the pending poll cursor to the start of the chunk index space.
    pub(super) fn wrap_pending_poll_idx(&mut self) {
        self.pending_poll_idx = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_cursor_advances_and_wraps() {
        let mut state = ChunkLifecycleState::default();
        assert_eq!(state.pending_poll_idx(), 0);

        state.advance_pending_poll_idx(Some(7));
        assert_eq!(state.pending_poll_idx(), 8);

        state.wrap_pending_poll_idx();
        assert_eq!(state.pending_poll_idx(), 0);
    }
}

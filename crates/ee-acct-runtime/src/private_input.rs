use strata_ee_acct_types::CommitChainSegment;

/// Additional private input accessible to verification fns.
#[derive(Clone, Debug)]
pub struct SharedPrivateInput {
    /// Implicit commits that we're processing.
    commit_data: Vec<CommitChainSegment>,

    /// Previous header that we already have in our state.
    raw_prev_header: Vec<u8>,

    /// Partial pre-state corresponding to the previous header.
    raw_partial_pre_state: Vec<u8>,
}

impl SharedPrivateInput {
    pub fn new(
        commit_data: Vec<CommitChainSegment>,
        raw_prev_header: Vec<u8>,
        raw_partial_pre_state: Vec<u8>,
    ) -> Self {
        Self {
            commit_data,
            raw_prev_header,
            raw_partial_pre_state,
        }
    }

    pub fn commit_data(&self) -> &[CommitChainSegment] {
        &self.commit_data
    }

    pub fn raw_prev_header(&self) -> &[u8] {
        &self.raw_prev_header
    }

    pub fn raw_partial_pre_state(&self) -> &[u8] {
        &self.raw_partial_pre_state
    }
}

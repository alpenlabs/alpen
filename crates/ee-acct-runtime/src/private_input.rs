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

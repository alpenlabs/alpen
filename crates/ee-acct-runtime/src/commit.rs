use strata_acct_types::Hash;

/// Data about a pending commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingCommit {
    new_tip_exec_blkid: Hash,
}

impl PendingCommit {
    pub(crate) fn new(new_tip_exec_blkid: Hash) -> Self {
        Self { new_tip_exec_blkid }
    }

    pub(crate) fn new_tip_exec_blkid(&self) -> Hash {
        self.new_tip_exec_blkid
    }
}

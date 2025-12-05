use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::OLBlockCommitment;

/// Additional metadata associated with the block.
/// Most of these can be derived from data in package or account_state, but are cached
/// here for ease of access.
#[derive(Debug, Clone)]
struct ExecPackageMetadata {
    /// Blocknumber of the exec chain block.
    blocknum: u64,
    /// Blockhash of the parent exec chain block.
    parent_blockhash: Hash,
    /// Timestamp of the exec block.
    timestamp_ms: u64,
    /// Commitment of the last ol chain block whose inbox messages were used in this exec block.
    ///
    /// Note:
    /// 1. `package.inputs` are derived according to this this ol block and previous exec block.
    /// 2. This does not uniquely identify a package or exec block. One `ol_block` can be linked
    ///    with multiple records.
    ol_block: OLBlockCommitment,
}

/// `ExecBlockPackage` with additional block metadata
#[derive(Debug, Clone)]
pub struct ExecBlockRecord {
    /// Additional metadata associated with this block.
    metadata: ExecPackageMetadata,
    /// The execution block package with additional block data.
    package: ExecBlockPackage,
    /// The final account state as a result of this execution.
    account_state: EeAccountState,
}

impl ExecBlockRecord {
    pub fn new(
        package: ExecBlockPackage,
        account_state: EeAccountState,
        blocknum: u64,
        ol_block: OLBlockCommitment,
        timestamp_ms: u64,
        parent_blockhash: Hash,
    ) -> Self {
        Self {
            package,
            account_state,
            metadata: ExecPackageMetadata {
                blocknum,
                ol_block,
                timestamp_ms,
                parent_blockhash,
            },
        }
    }

    pub fn package(&self) -> &ExecBlockPackage {
        &self.package
    }

    pub fn account_state(&self) -> &EeAccountState {
        &self.account_state
    }

    pub fn blocknum(&self) -> u64 {
        self.metadata.blocknum
    }

    pub fn ol_block(&self) -> &OLBlockCommitment {
        &self.metadata.ol_block
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.metadata.timestamp_ms
    }

    pub fn blockhash(&self) -> Hash {
        self.account_state.last_exec_blkid()
    }

    pub fn parent_blockhash(&self) -> Hash {
        self.metadata.parent_blockhash
    }

    pub fn into_parts(self) -> (ExecBlockPackage, EeAccountState) {
        (self.package, self.account_state)
    }
}

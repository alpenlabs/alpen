use alloy_primitives::B256;
use strata_acct_types::AccountId;

/// Chain specific config, that needs to remain constant on all nodes
/// to ensure all stay on the same chain.
#[derive(Debug, Clone)]
pub struct AlpenEeParams {
    /// Account id of current EE in OL
    account_id: AccountId,

    /// Genesis blockhash of execution chain
    genesis_blockhash: B256,

    /// Genesis stateroot of execution chain
    genesis_stateroot: B256,

    /// Block number of execution chain genesis block
    /// This can potentially be non-zero, but is very unlikely.
    genesis_blocknum: u64,
}

impl AlpenEeParams {
    /// Creates new chain parameters.
    pub fn new(
        account_id: AccountId,
        genesis_blockhash: B256,
        genesis_stateroot: B256,
        genesis_blocknum: u64,
    ) -> Self {
        Self {
            account_id,
            genesis_blockhash,
            genesis_stateroot,
            genesis_blocknum,
        }
    }

    /// Returns the EE account ID in the OL chain.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns the genesis block hash of the execution chain.
    pub fn genesis_blockhash(&self) -> B256 {
        self.genesis_blockhash
    }

    /// Returns the genesis state root of the execution chain.
    pub fn genesis_stateroot(&self) -> B256 {
        self.genesis_stateroot
    }

    /// Returns the genesis block number of the execution chain.
    pub fn genesis_blocknum(&self) -> u64 {
        self.genesis_blocknum
    }
}

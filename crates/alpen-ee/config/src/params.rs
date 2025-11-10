use alloy_primitives::B256;
use strata_acct_types::AccountId;
use strata_identifiers::OLBlockId;

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

    /// OL slot of Alpen ee account genesis
    genesis_ol_slot: u64,

    /// Ol block of Alpen ee account genesis
    genesis_ol_blockid: OLBlockId,
}

impl AlpenEeParams {
    pub fn new(
        account_id: AccountId,
        genesis_blockhash: B256,
        genesis_stateroot: B256,
        genesis_ol_slot: u64,
        genesis_ol_blockid: OLBlockId,
    ) -> Self {
        Self {
            account_id,
            genesis_blockhash,
            genesis_stateroot,
            genesis_ol_slot,
            genesis_ol_blockid,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn genesis_blockhash(&self) -> B256 {
        self.genesis_blockhash
    }

    pub fn genesis_stateroot(&self) -> B256 {
        self.genesis_stateroot
    }

    pub fn genesis_ol_slot(&self) -> u64 {
        self.genesis_ol_slot
    }

    pub fn genesis_ol_blockid(&self) -> OLBlockId {
        self.genesis_ol_blockid
    }
}

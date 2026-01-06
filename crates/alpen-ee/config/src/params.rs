use alloy_primitives::B256;
use strata_acct_types::AccountId;
use strata_identifiers::{EpochCommitment, OLBlockId};

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

    /// OL Epoch of Alpen ee account genesis
    genesis_ol_epoch: u32,

    /// OL slot of Alpen ee account genesis
    genesis_ol_slot: u64,

    /// OL block of Alpen ee account genesis
    genesis_ol_blockid: OLBlockId,
}

impl AlpenEeParams {
    /// Creates new chain parameters.
    pub fn new(
        account_id: AccountId,
        genesis_blockhash: B256,
        genesis_stateroot: B256,
        genesis_blocknum: u64,
        genesis_ol_epoch: u32,
        genesis_ol_slot: u64,
        genesis_ol_blockid: OLBlockId,
    ) -> Self {
        Self {
            account_id,
            genesis_blockhash,
            genesis_stateroot,
            genesis_blocknum,
            genesis_ol_epoch,
            genesis_ol_slot,
            genesis_ol_blockid,
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

    /// Returns the genesis block hash of the execution chain.
    pub fn genesis_blocknum(&self) -> u64 {
        self.genesis_blocknum
    }

    /// Returns the OL slot number at genesis.
    pub fn genesis_ol_slot(&self) -> u64 {
        self.genesis_ol_slot
    }

    pub fn genesis_ol_epoch(&self) -> u32 {
        self.genesis_ol_epoch
    }

    /// Returns the OL block ID at genesis.
    pub fn genesis_ol_blockid(&self) -> OLBlockId {
        self.genesis_ol_blockid
    }

    pub fn genesis_ol_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment {
            epoch: self.genesis_ol_epoch,
            last_slot: self.genesis_ol_slot,
            last_blkid: self.genesis_ol_blockid,
        }
    }
}

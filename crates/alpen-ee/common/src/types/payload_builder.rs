use alloy_primitives::{Address, B256};
use strata_acct_types::BitcoinAmount;

/// Inputs to control evm block builder.
#[derive(Debug, Clone)]
pub struct PayloadBuildAttributes {
    /// blockhash of parent block for new block.
    parent: B256,
    /// timestamp of the new block.
    timestamp: u64,
    /// deposits to be included in the new block.
    deposits: Vec<DepositInfo>,
}

impl PayloadBuildAttributes {
    pub fn new(parent: B256, timestamp: u64, deposits: Vec<DepositInfo>) -> Self {
        Self {
            parent,
            timestamp,
            deposits,
        }
    }

    pub fn parent(&self) -> B256 {
        self.parent
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn deposits(&self) -> &[DepositInfo] {
        &self.deposits
    }
}

/// Describes an incoming deposit that should be minted.
#[derive(Debug, Clone)]
pub struct DepositInfo {
    /// Address inside evm chain where the deposit should be minted to.
    address: Address,
    /// Amount that has been deposited.
    amount: BitcoinAmount,
}

impl DepositInfo {
    pub fn new(address: Address, amount: BitcoinAmount) -> Self {
        Self { address, amount }
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn amount(&self) -> BitcoinAmount {
        self.amount
    }
}

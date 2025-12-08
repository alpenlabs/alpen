use alloy_primitives::{Address, B256};
use strata_acct_types::BitcoinAmount;

#[derive(Debug, Clone)]
pub struct PayloadBuildAttributes {
    parent: B256,
    timestamp: u64,
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

#[derive(Debug, Clone)]
pub struct DepositInfo {
    index: u64,
    address: Address,
    amount: BitcoinAmount,
}

impl DepositInfo {
    pub fn new(index: u64, address: Address, amount: BitcoinAmount) -> Self {
        Self {
            index,
            address,
            amount,
        }
    }

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn amount(&self) -> BitcoinAmount {
        self.amount
    }
}

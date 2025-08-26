use std::ops::Deref;

use alloy_consensus::constants::ETH_TO_WEI;
use alloy_primitives::{Address, FixedBytes, U256};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountId(FixedBytes<32>);

impl From<FixedBytes<32>> for AccountId {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct L1BlockId(FixedBytes<32>);

impl From<FixedBytes<32>> for L1BlockId {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

impl L1BlockId {
    pub fn zero() -> Self {
        Self(FixedBytes::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OlBlockId(FixedBytes<32>);

impl OlBlockId {
    pub fn zero() -> Self {
        Self(FixedBytes::ZERO)
    }
}

impl From<FixedBytes<32>> for OlBlockId {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

const BTC_TO_WEI: u128 = ETH_TO_WEI;
pub const SATS_TO_WEI: u128 = BTC_TO_WEI / 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    sats: u64,
}

impl Balance {
    pub fn from_sats(sats: u64) -> Self {
        Self { sats }
    }

    pub fn from_wei(wei: U256) -> Self {
        let (sats, rem) = wei.div_rem(U256::from(SATS_TO_WEI));
        assert_eq!(rem, U256::ZERO);
        Self { sats: sats.to() }
    }

    pub fn to_wei(&self) -> U256 {
        U256::from(self.sats) * U256::from(SATS_TO_WEI)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountStateCommitment(FixedBytes<32>);

impl AccountStateCommitment {
    pub fn zero() -> Self {
        Self(FixedBytes::ZERO)
    }

    pub fn inner(&self) -> FixedBytes<32> {
        self.0
    }
}

impl From<FixedBytes<32>> for AccountStateCommitment {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

impl Deref for AccountStateCommitment {
    type Target = FixedBytes<32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub enum InputMsg {
    Deposit {
        /// internal EE Address
        to: Address,
        amount: Balance,
    },
}

#[derive(Debug, Clone)]
pub enum OutputMsg {
    Withdrawal {
        /// internal EE Address
        from: Address,
        /// L1 Address
        to: FixedBytes<32>,
        amount: Balance,
    },
}

#[derive(Debug, Clone)]
pub struct EEAccount {
    pub(crate) sequence_no: u64,
    pub(crate) balance: Balance,
    pub(crate) state_commitment: AccountStateCommitment,
    pub(crate) messages: Vec<InputMsg>,
}

impl EEAccount {
    pub fn state_commitment(&self) -> &AccountStateCommitment {
        &self.state_commitment
    }

    pub fn into_parts(self) -> (u64, Balance, AccountStateCommitment, Vec<InputMsg>) {
        (
            self.sequence_no,
            self.balance,
            self.state_commitment,
            self.messages,
        )
    }
}

#[derive(Debug, Clone, Default)]
struct Proof(Vec<u8>);

#[derive(Debug, Clone)]
pub struct EEUpdate {
    pub(crate) sequence_no: u64,
    pub(crate) from_state: AccountStateCommitment,
    pub(crate) to_state: AccountStateCommitment,
    pub(crate) messages: Vec<OutputMsg>,
    pub(crate) proof: Proof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L1BlockCommitment {
    blockhash: L1BlockId,
    height: u64,
}

impl L1BlockCommitment {
    pub fn new(blockhash: L1BlockId, height: u64) -> Self {
        Self { blockhash, height }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn blockhash(&self) -> &L1BlockId {
        &self.blockhash
    }

    pub fn into_parts(self) -> (L1BlockId, u64) {
        (self.blockhash, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OlBlockCommitment {
    blockhash: OlBlockId,
    slot: u64,
    l1_commitment: L1BlockCommitment,
}

impl OlBlockCommitment {
    pub fn new(blockhash: OlBlockId, slot: u64, l1_commitment: L1BlockCommitment) -> Self {
        Self {
            blockhash,
            slot,
            l1_commitment,
        }
    }

    pub fn into_parts(self) -> (OlBlockId, u64, L1BlockCommitment) {
        (self.blockhash, self.slot, self.l1_commitment)
    }

    pub fn blockhash(&self) -> &OlBlockId {
        &self.blockhash
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn l1_commitment(&self) -> &L1BlockCommitment {
        &self.l1_commitment
    }
}

#[derive(Debug, Clone)]
pub enum ConsensusEvent {
    Head(AccountStateCommitment),
    OlUpdated {
        confirmed: AccountStateCommitment,
        finalized: AccountStateCommitment,
    },
}

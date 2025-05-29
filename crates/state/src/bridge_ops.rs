//! Types for managing pending bridging operations in the CL state.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32, l1::BitcoinAmount};

// TODO make this not hardcoded!
pub const WITHDRAWAL_DENOMINATION: BitcoinAmount = BitcoinAmount::from_int_btc(10);

/// Describes an intent to withdraw that hasn't been dispatched yet.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalIntent {
    /// Quantity of L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination [`Descriptor`] for the withdrawal
    destination: Descriptor,

    /// withdrawal request transaction id
    withdrawal_txid: Buf32,
}

impl WithdrawalIntent {
    pub const fn new(amt: BitcoinAmount, destination: Descriptor, withdrawal_txid: Buf32) -> Self {
        Self {
            amt,
            destination,
            withdrawal_txid,
        }
    }

    pub const fn as_parts(&self) -> (u64, &Descriptor) {
        (self.amt.to_sat(), &self.destination)
    }

    pub const fn amt(&self) -> &BitcoinAmount {
        &self.amt
    }

    pub const fn destination(&self) -> &Descriptor {
        &self.destination
    }

    pub const fn withdrawal_txid(&self) -> &Buf32 {
        &self.withdrawal_txid
    }
}

/// Set of withdrawals that are assigned to a deposit bridge utxo.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalBatch {
    /// A series of [WithdrawalIntent]'s who sum does not exceed [`WITHDRAWAL_DENOMINATION`].
    intents: Vec<WithdrawalIntent>,
}

impl WithdrawalBatch {
    /// Creates a new instance.
    pub const fn new(intents: Vec<WithdrawalIntent>) -> Self {
        Self { intents }
    }

    /// Gets the total value of the batch.  This must be less than the size of
    /// the utxo it's assigned to.
    pub fn get_total_value(&self) -> BitcoinAmount {
        self.intents.iter().map(|wi| wi.amt).sum()
    }

    pub fn intents(&self) -> &[WithdrawalIntent] {
        &self.intents[..]
    }
}

/// Describes a deposit data to be processed by an EE.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositIntent {
    /// Quantity in the L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Description of the encoded address. For EVM this is the 20-byte
    /// address.
    dest_ident: Vec<u8>,
}

impl DepositIntent {
    pub const fn new(amt: BitcoinAmount, dest_ident: Vec<u8>) -> Self {
        Self { amt, dest_ident }
    }

    pub const fn amt(&self) -> u64 {
        self.amt.to_sat()
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn dest_ident(&self) -> &[u8] {
        &self.dest_ident
    }
}

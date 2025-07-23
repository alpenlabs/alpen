//! Withdrawal-related types and commands.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

/// Command to operator(s) to initiate the withdrawal.  Describes the set of
/// outputs we're trying to withdraw to.
///
/// May also include future information to deal with fee accounting.
///
/// # Note
///
/// This is mostly here in order to support withdrawal batching (i.e., sub-denomination withdrawal
/// amounts that can be batched and then serviced together). At the moment, the underlying `Vec` of
/// [`WithdrawOutput`] always has a single element.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalCommand {
    /// The table of withdrawal outputs.
    withdraw_outputs: Vec<WithdrawOutput>,
}

impl WithdrawalCommand {
    pub fn new(withdraw_outputs: Vec<WithdrawOutput>) -> Self {
        Self { withdraw_outputs }
    }

    pub fn withdraw_outputs(&self) -> &[WithdrawOutput] {
        &self.withdraw_outputs
    }
}

/// An output constructed from [`crate::bridge_ops::WithdrawalIntent`].
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WithdrawOutput {
    /// BOSD [`Descriptor`].
    destination: Descriptor,

    /// Amount in sats.
    amt: BitcoinAmount,
}

impl WithdrawOutput {
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}
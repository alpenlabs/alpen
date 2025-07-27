//! Withdrawal Command Management
//!
//! This module contains types for specifying withdrawal commands and outputs.
//! Withdrawal commands define the Bitcoin outputs that operators should create
//! when processing withdrawal requests from deposits.

use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bitcoin_bosd::Descriptor, bridge::OperatorIdx, buf::Buf32, l1::BitcoinAmount,
};

/// Command specifying Bitcoin outputs for a withdrawal operation.
///
/// This structure instructs operators on how to construct the Bitcoin transaction
/// outputs when processing a withdrawal. Each command contains a list of outputs
/// with their destinations and amounts.
///
/// # Batching Support
///
/// The design supports withdrawal batching where multiple sub-denomination amounts
/// can be combined and processed together in a single transaction. Currently,
/// withdrawal commands contain a single output, but the structure is prepared for
/// future batching implementations.
///
/// # Fee Handling
///
/// Future versions may include additional fee accounting information to help
/// operators calculate appropriate transaction fees.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct WithdrawalCommand {
    /// List of Bitcoin outputs to create in the withdrawal transaction.
    withdraw_outputs: Vec<WithdrawOutput>,
}

impl WithdrawalCommand {
    /// Creates a new withdrawal command with the specified outputs.
    ///
    /// # Parameters
    ///
    /// - `withdraw_outputs` - Vector of withdrawal outputs specifying destinations and amounts
    ///
    /// # Returns
    ///
    /// A new [`WithdrawalCommand`] instance.
    pub fn new(withdraw_outputs: Vec<WithdrawOutput>) -> Self {
        Self { withdraw_outputs }
    }

    /// Returns a slice of all withdrawal outputs.
    ///
    /// # Returns
    ///
    /// Slice reference to all [`WithdrawOutput`] instances in this command.
    pub fn withdraw_outputs(&self) -> &[WithdrawOutput] {
        &self.withdraw_outputs
    }

    /// Gets the total value of the batch.  This must be less than the size of
    /// the utxo it's assigned to.
    pub fn get_total_value(&self) -> BitcoinAmount {
        self.withdraw_outputs.iter().map(|wi| wi.amt).sum()
    }
}

/// Bitcoin output specification for a withdrawal operation.
///
/// Each withdrawal output specifies a destination address (as a Bitcoin descriptor)
/// and the amount to be sent. This structure provides all information needed by
/// operators to construct the appropriate Bitcoin transaction output.
///
/// # Bitcoin Descriptors
///
/// The destination uses Bitcoin Output Script Descriptors (BOSD) which provide
/// a standardized way to specify Bitcoin addresses and locking conditions.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WithdrawOutput {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    destination: Descriptor,

    /// Amount to withdraw (in satoshis).
    amt: BitcoinAmount,
}

impl WithdrawOutput {
    /// Creates a new withdrawal output with the specified destination and amount.
    ///
    /// # Parameters
    ///
    /// - `destination` - Bitcoin descriptor specifying the destination address
    /// - `amt` - Amount to withdraw in satoshis
    ///
    /// # Returns
    ///
    /// A new [`WithdrawOutput`] instance.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    /// Returns a reference to the destination descriptor.
    ///
    /// # Returns
    ///
    /// Reference to the [`Descriptor`] specifying where funds should be sent.
    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    /// Returns the withdrawal amount.
    ///
    /// # Returns
    ///
    /// The withdrawal amount as [`BitcoinAmount`] (in satoshis).
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

/// Information about a successfully processed withdrawal.
///
/// This structure holds the essential information from a withdrawal transaction
/// that needs to be stored in the MohoState for later use by the Bridge proof.
/// The Bridge proof uses this information to prove that operators have correctly
/// front-paid users and can now withdraw the corresponding locked funds.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct WithdrawalProcessedInfo {
    /// The transaction ID of the withdrawal transaction
    pub withdrawal_txid: Buf32,

    /// The transaction ID of the deposit that was assigned
    pub deposit_txid: Buf32,

    /// The transaction idx of the deposit that was assigned
    pub deposit_idx: u32,

    /// The index of the operator who processed the withdrawal
    pub operator_idx: OperatorIdx,
}

impl WithdrawalProcessedInfo {
    pub fn to_export_entry(&self) -> ExportEntry {
        let payload = borsh::to_vec(&self).expect("Failed to serialize WithdrawalProcessedInfo");
        ExportEntry::new(self.deposit_idx, payload)
    }
}

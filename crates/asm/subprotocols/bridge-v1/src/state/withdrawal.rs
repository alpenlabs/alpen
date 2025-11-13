//! Withdrawal Command Management
//!
//! This module contains types for specifying withdrawal commands and requests.
//! These types define the Bitcoin outputs that operators should create when
//! processing withdrawal requests from deposits.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use serde::{Deserialize, Serialize};
use strata_bridge_types::OperatorIdx;
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    l1::{BitcoinAmount, BitcoinTxid},
};

/// Withdrawal request received from the Checkpoint subprotocol.
///
/// This structure represents a user's request to withdraw funds from the bridge.
/// It contains the destination address and the amount requested. The actual amount
/// received by the user will be less due to operator fees being deducted during
/// conversion to [`WithdrawalCommand`].
#[derive(
    Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Arbitrary,
)]
pub struct WithdrawalRequest {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    pub destination: Descriptor,

    /// Amount to withdraw (in satoshis), before fee deduction.
    pub amt: BitcoinAmount,
}

impl WithdrawalRequest {
    /// Creates a new withdrawal request with the specified destination and amount.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    /// Returns a reference to the destination descriptor.
    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    /// Returns the withdrawal amount (before fee deduction).
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }

    /// Converts this withdrawal request into a [`WithdrawalCommand`] by deducting the operator fee.
    ///
    /// The resulting command contains the net amount that will be sent to the user
    /// (requested amount minus operator fee).
    ///
    /// # Parameters
    ///
    /// - `operator_fee` - The fee amount to deduct from the requested amount
    ///
    /// # Returns
    ///
    /// A [`WithdrawalCommand`] with the net amount after fee deduction
    pub fn into_cmd(self, operator_fee: BitcoinAmount) -> WithdrawalCommand {
        let net_amount = self.amt.saturating_sub(operator_fee);
        WithdrawalCommand {
            destination: self.destination,
            amt: net_amount,
        }
    }
}

/// Withdrawal command specifying the user destination and net amount user gets after fee deduction.
///
/// This structure is created from a [`WithdrawalRequest`] by deducting the operator fee
/// from the requested amount. It represents the actual Bitcoin output that need to be
/// in the withdrawal transaction, with the net amount that the user will receive for a withdrawal
/// to be valid.
#[derive(
    Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Arbitrary,
)]
pub struct WithdrawalCommand {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    destination: Descriptor,

    /// Net amount to withdraw after operator fee deduction (in satoshis).
    amt: BitcoinAmount,
}

impl WithdrawalCommand {
    /// Creates a new withdrawal command with the specified destination and net amount.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    /// Returns a reference to the destination descriptor.
    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    /// Returns the withdrawal amount
    pub fn amount(&self) -> BitcoinAmount {
        self.amt
    }
}

/// Represents an operator's claim to unlock a deposit UTXO after successful withdrawal processing.
///
/// This structure is created when an operator successfully processes a withdrawal by making
/// the required front payment to the user within the specified deadline. It serves as proof
/// that the operator has fulfilled their obligation and is now entitled to claim the
/// corresponding locked deposit funds.
///
/// The claim contains all necessary information to:
/// - Link the withdrawal transaction to the original deposit
/// - Identify which operator performed the withdrawal
/// - Enable the Bridge proof to verify the operator's right to withdraw locked funds
///
/// This data is stored in the MohoState and used by the Bridge proof system to validate
/// that operators have correctly front-paid users before allowing them to withdraw the
/// corresponding deposit UTXOs.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct OperatorClaimUnlock {
    /// The transaction ID of the withdrawal transaction
    pub withdrawal_txid: BitcoinTxid,

    /// The transaction ID of the deposit that was assigned
    pub deposit_txid: BitcoinTxid,

    /// The transaction idx of the deposit that was assigned
    pub deposit_idx: u32,

    /// The index of the operator who processed the withdrawal
    pub operator_idx: OperatorIdx,
}

impl OperatorClaimUnlock {
    pub fn to_export_entry(&self) -> ExportEntry {
        let payload = borsh::to_vec(&self).expect("Failed to serialize WithdrawalProcessedInfo");
        ExportEntry::new(self.deposit_idx, payload)
    }
}

//! Withdrawal Command Management
//!
//! This module contains types for specifying withdrawal commands and outputs.
//! Withdrawal commands define the Bitcoin outputs that operators should create
//! when processing withdrawal requests from deposits.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use serde::{Deserialize, Serialize};
use strata_bridge_types::OperatorIdx;
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    l1::{BitcoinAmount, BitcoinTxid},
};

/// Command specifying a Bitcoin output for a withdrawal operation.
///
/// This structure instructs operators on how to construct the Bitcoin transaction
/// output when processing a withdrawal. It currently contains a single output specifying the
/// destination and amount, along with the operator fee that will be deducted.
///
/// ## Fee Structure
///
/// The operator fee is deducted from the withdrawal amount before creating the Bitcoin
/// output. This means the user receives the net amount (withdrawal amount minus operator
/// fee) in their Bitcoin transaction, while the operator keeps the fee as compensation
/// for processing the withdrawal.
///
/// ## Future Enhancements
///
/// - **Batching**: Support for multiple outputs in a single withdrawal command to enable efficient
///   processing of multiple withdrawals in one Bitcoin transaction
#[derive(
    Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Arbitrary,
)]
pub struct WithdrawalCommand {
    /// Bitcoin output to create in the withdrawal transaction.
    output: WithdrawOutput,

    /// Amount the operator can take as fees for processing withdrawal.
    operator_fee: BitcoinAmount,
}

impl WithdrawalCommand {
    /// Creates a new withdrawal command with the specified output and operator fee.
    pub fn new(output: WithdrawOutput, operator_fee: BitcoinAmount) -> Self {
        Self {
            output,
            operator_fee,
        }
    }

    /// Returns a reference to the destination descriptor for this withdrawal.
    pub fn destination(&self) -> &Descriptor {
        &self.output.destination
    }

    /// Updates the operator fee for this withdrawal command.
    pub fn update_fee(&mut self, new_fee: BitcoinAmount) {
        self.operator_fee = new_fee
    }

    /// Calculates the net amount the user will receive after operator fee deduction.
    ///
    /// This is the amount that will actually be sent to the user's Bitcoin address,
    /// which equals the withdrawal amount minus the operator fee.
    pub fn net_amount(&self) -> BitcoinAmount {
        self.output.amt().saturating_sub(self.operator_fee)
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
#[derive(
    Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Arbitrary,
)]
pub struct WithdrawOutput {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    pub destination: Descriptor,

    /// Amount to withdraw (in satoshis).
    pub amt: BitcoinAmount,
}

impl WithdrawOutput {
    /// Creates a new withdrawal output with the specified destination and amount.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    /// Returns a reference to the destination descriptor.
    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    /// Returns the withdrawal amount.
    pub fn amt(&self) -> BitcoinAmount {
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
    /// The transaction ID of the withdrawal fulfillment transaction, i.e. the transaction which
    /// fronts funds to the user.
    pub fulfillment_txid: BitcoinTxid,

    /// The transaction ID of the deposit that was assigned.
    pub deposit_txid: BitcoinTxid,

    /// The transaction idx of the deposit that was assigned.
    pub deposit_idx: u32,

    /// The index of the operator who processed the withdrawal.
    pub operator_idx: OperatorIdx,
}

impl OperatorClaimUnlock {
    pub fn to_export_entry(&self) -> ExportEntry {
        let payload = borsh::to_vec(&self).expect("Failed to serialize WithdrawalProcessedInfo");
        ExportEntry::new(self.deposit_idx, payload)
    }
}

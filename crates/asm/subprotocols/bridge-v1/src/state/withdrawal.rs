//! Withdrawal Command Management
//!
//! This module contains types for specifying withdrawal commands and outputs.
//! Withdrawal commands define the Bitcoin outputs that operators should create
//! when processing withdrawal requests from deposits.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_asm_bridge_msgs::WithdrawOutput;
use strata_bridge_types::OperatorIdx;
use strata_codec::{Codec, encode_to_vec};
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    hash::{self},
    l1::BitcoinAmount,
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

/// Represents an operator's claim to unlock a deposit UTXO after successful withdrawal fulfillment.
///
/// This structure is created when a withdrawal fulfillment transaction is successfully validated.
/// It serves as proof that a valid frontpayment was made matching the assignment specifications,
/// and authorizes the assigned operator to claim the corresponding locked deposit funds through
/// the Bridge proof system.
///
/// The claim contains:
/// - The deposit index that identifies which locked UTXO can be claimed
/// - The operator index of the assigned operator who is authorized to claim
///
/// # Important Notes
///
/// - The `operator_idx` always refers to the **assigned operator** from the assignment entry, not
///   necessarily the party who made the actual frontpayment (since frontpayment identity is not
///   validated during transaction processing).
/// - This data is stored in the MohoState and emitted as an ASM log via `NewExportEntry`.
/// - The Bridge proof system consumes these entries to verify operators have correctly fulfilled
///   withdrawal obligations before allowing them to unlock deposit UTXOs.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Codec)]
pub struct OperatorClaimUnlock {
    /// The index of the deposit that was fulfilled.
    pub deposit_idx: u32,

    /// The index of the operator who was assigned to (and is authorized to claim) this withdrawal.
    pub operator_idx: OperatorIdx,
}

impl OperatorClaimUnlock {
    pub fn compute_hash(&self) -> [u8; 32] {
        let buf = encode_to_vec(self).expect("failed to encode OperatorClaimUnlock");
        hash::raw(&buf).0
    }
}

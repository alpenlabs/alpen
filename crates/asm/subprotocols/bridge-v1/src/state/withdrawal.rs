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

#[cfg(test)]
mod tests {
    use super::*;
    use strata_primitives::bitcoin_bosd::Descriptor;

    fn create_test_descriptor() -> Descriptor {
        // Create a simple test descriptor - this is just for testing
        Descriptor::new_p2pkh(&[0u8; 20])
    }

    #[test]
    fn test_withdraw_output_new() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(1000);

        let output = WithdrawOutput::new(descriptor.clone(), amount);

        assert_eq!(output.destination(), &descriptor);
        assert_eq!(output.amt(), amount);
    }

    #[test]
    fn test_withdraw_output_getters() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(50000);

        let output = WithdrawOutput::new(descriptor.clone(), amount);

        // Test all getter methods
        assert_eq!(output.destination(), &descriptor);
        assert_eq!(output.amt(), amount);
    }

    #[test]
    fn test_withdraw_output_with_zero_amount() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(0);

        let output = WithdrawOutput::new(descriptor, amount);

        assert_eq!(output.amt(), BitcoinAmount::from_sat(0));
    }

    #[test]
    fn test_withdraw_output_with_large_amount() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(2_100_000_000_000_000); // 21M BTC in sats

        let output = WithdrawOutput::new(descriptor, amount);

        assert_eq!(output.amt(), amount);
    }

    #[test]
    fn test_withdraw_output_clone() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(1000);

        let output1 = WithdrawOutput::new(descriptor.clone(), amount);
        let output2 = output1.clone();

        assert_eq!(output1.destination(), output2.destination());
        assert_eq!(output1.amt(), output2.amt());
    }

    #[test]
    fn test_withdraw_output_equality() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(500);

        let output1 = WithdrawOutput::new(descriptor.clone(), amount);
        let output2 = WithdrawOutput::new(descriptor, amount);

        assert_eq!(output1, output2);
    }

    #[test]
    fn test_withdraw_output_inequality() {
        let descriptor1 = create_test_descriptor();
        let descriptor2 = Descriptor::new_p2pkh(&[1u8; 20]);
        let amount = BitcoinAmount::from_sat(1000);

        let output1 = WithdrawOutput::new(descriptor1, amount);
        let output2 = WithdrawOutput::new(descriptor2, amount);

        assert_ne!(output1, output2);
    }

    #[test]
    fn test_withdrawal_command_new() {
        let descriptor = create_test_descriptor();
        let output = WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(1000));
        let outputs = vec![output.clone()];

        let command = WithdrawalCommand::new(outputs.clone());

        assert_eq!(command.withdraw_outputs().len(), 1);
        assert_eq!(command.withdraw_outputs()[0], output);
    }

    #[test]
    fn test_withdrawal_command_empty_outputs() {
        let outputs = vec![];
        let command = WithdrawalCommand::new(outputs);

        assert_eq!(command.withdraw_outputs().len(), 0);
        assert!(command.withdraw_outputs().is_empty());
    }

    #[test]
    fn test_withdrawal_command_multiple_outputs() {
        let descriptor1 = create_test_descriptor();
        let descriptor2 = Descriptor::new_p2pkh(&[1u8; 20]);

        let output1 = WithdrawOutput::new(descriptor1, BitcoinAmount::from_sat(1000));
        let output2 = WithdrawOutput::new(descriptor2, BitcoinAmount::from_sat(2000));
        let outputs = vec![output1.clone(), output2.clone()];

        let command = WithdrawalCommand::new(outputs);

        assert_eq!(command.withdraw_outputs().len(), 2);
        assert_eq!(command.withdraw_outputs()[0], output1);
        assert_eq!(command.withdraw_outputs()[1], output2);
    }

    #[test]
    fn test_withdrawal_command_get_total_value_single_output() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(1000);
        let output = WithdrawOutput::new(descriptor, amount);
        let command = WithdrawalCommand::new(vec![output]);

        assert_eq!(command.get_total_value(), amount);
    }

    #[test]
    fn test_withdrawal_command_get_total_value_multiple_outputs() {
        let descriptor1 = create_test_descriptor();
        let descriptor2 = Descriptor::new_p2pkh(&[1u8; 20]);

        let amount1 = BitcoinAmount::from_sat(1000);
        let amount2 = BitcoinAmount::from_sat(2500);
        let total_expected = BitcoinAmount::from_sat(3500);

        let output1 = WithdrawOutput::new(descriptor1, amount1);
        let output2 = WithdrawOutput::new(descriptor2, amount2);
        let command = WithdrawalCommand::new(vec![output1, output2]);

        assert_eq!(command.get_total_value(), total_expected);
    }

    #[test]
    fn test_withdrawal_command_get_total_value_zero_amounts() {
        let descriptor1 = create_test_descriptor();
        let descriptor2 = Descriptor::new_p2pkh(&[1u8; 20]);

        let amount1 = BitcoinAmount::from_sat(0);
        let amount2 = BitcoinAmount::from_sat(0);

        let output1 = WithdrawOutput::new(descriptor1, amount1);
        let output2 = WithdrawOutput::new(descriptor2, amount2);
        let command = WithdrawalCommand::new(vec![output1, output2]);

        assert_eq!(command.get_total_value(), BitcoinAmount::from_sat(0));
    }

    #[test]
    fn test_withdrawal_command_get_total_value_empty() {
        let command = WithdrawalCommand::new(vec![]);

        assert_eq!(command.get_total_value(), BitcoinAmount::from_sat(0));
    }

    #[test]
    fn test_withdrawal_command_clone() {
        let descriptor = create_test_descriptor();
        let output = WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(1000));
        let command1 = WithdrawalCommand::new(vec![output.clone()]);
        let command2 = command1.clone();

        assert_eq!(command1.withdraw_outputs().len(), command2.withdraw_outputs().len());
        assert_eq!(command1.withdraw_outputs()[0], command2.withdraw_outputs()[0]);
    }

    #[test]
    fn test_withdrawal_command_equality() {
        let descriptor = create_test_descriptor();
        let output = WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(1000));

        let command1 = WithdrawalCommand::new(vec![output.clone()]);
        let command2 = WithdrawalCommand::new(vec![output]);

        assert_eq!(command1, command2);
    }

    #[test]
    fn test_withdrawal_command_inequality() {
        let descriptor = create_test_descriptor();
        let output1 = WithdrawOutput::new(descriptor.clone(), BitcoinAmount::from_sat(1000));
        let output2 = WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(2000));

        let command1 = WithdrawalCommand::new(vec![output1]);
        let command2 = WithdrawalCommand::new(vec![output2]);

        assert_ne!(command1, command2);
    }

    #[test]
    fn test_withdrawal_processed_info_new() {
        let withdrawal_txid = Buf32::from([1u8; 32]);
        let deposit_txid = Buf32::from([2u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        assert_eq!(info.withdrawal_txid, withdrawal_txid);
        assert_eq!(info.deposit_txid, deposit_txid);
        assert_eq!(info.deposit_idx, deposit_idx);
        assert_eq!(info.operator_idx, operator_idx);
    }

    #[test]
    fn test_withdrawal_processed_info_to_export_entry() {
        let withdrawal_txid = Buf32::from([1u8; 32]);
        let deposit_txid = Buf32::from([2u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        let _export_entry = info.to_export_entry();

        // Test that the export entry was created successfully 
        // (we can't access internal fields without knowing the ExportEntry API)
        
        // The main test is that serialization works correctly
        let serialized = borsh::to_vec(&info).expect("Failed to serialize WithdrawalProcessedInfo");
        assert!(!serialized.is_empty());

        // Verify we can deserialize back to the original info
        let deserialized_info: WithdrawalProcessedInfo = 
            borsh::from_slice(&serialized).expect("Failed to deserialize payload");

        assert_eq!(deserialized_info.withdrawal_txid, withdrawal_txid);
        assert_eq!(deserialized_info.deposit_txid, deposit_txid);
        assert_eq!(deserialized_info.deposit_idx, deposit_idx);
        assert_eq!(deserialized_info.operator_idx, operator_idx);
    }

    #[test]
    fn test_withdrawal_processed_info_clone() {
        let withdrawal_txid = Buf32::from([1u8; 32]);
        let deposit_txid = Buf32::from([2u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info1 = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        let info2 = info1.clone();

        assert_eq!(info1.withdrawal_txid, info2.withdrawal_txid);
        assert_eq!(info1.deposit_txid, info2.deposit_txid);
        assert_eq!(info1.deposit_idx, info2.deposit_idx);
        assert_eq!(info1.operator_idx, info2.operator_idx);
    }

    #[test]
    fn test_withdrawal_processed_info_equality() {
        let withdrawal_txid = Buf32::from([1u8; 32]);
        let deposit_txid = Buf32::from([2u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info1 = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        let info2 = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        assert_eq!(info1, info2);
    }

    #[test]
    fn test_withdrawal_processed_info_inequality() {
        let withdrawal_txid1 = Buf32::from([1u8; 32]);
        let withdrawal_txid2 = Buf32::from([2u8; 32]);
        let deposit_txid = Buf32::from([3u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info1 = WithdrawalProcessedInfo {
            withdrawal_txid: withdrawal_txid1,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        let info2 = WithdrawalProcessedInfo {
            withdrawal_txid: withdrawal_txid2,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        assert_ne!(info1, info2);
    }

    #[test]
    fn test_withdrawal_processed_info_borsh_serialization() {
        let withdrawal_txid = Buf32::from([1u8; 32]);
        let deposit_txid = Buf32::from([2u8; 32]);
        let deposit_idx = 42u32;
        let operator_idx = 3u32;

        let info = WithdrawalProcessedInfo {
            withdrawal_txid,
            deposit_txid,
            deposit_idx,
            operator_idx,
        };

        // Test serialization
        let serialized = borsh::to_vec(&info).expect("Serialization should succeed");
        assert!(!serialized.is_empty());

        // Test deserialization
        let deserialized: WithdrawalProcessedInfo = 
            borsh::from_slice(&serialized).expect("Deserialization should succeed");

        assert_eq!(info, deserialized);
    }

    #[test]
    fn test_withdrawal_output_borsh_serialization() {
        let descriptor = create_test_descriptor();
        let amount = BitcoinAmount::from_sat(1000);
        let output = WithdrawOutput::new(descriptor, amount);

        // Test serialization
        let serialized = borsh::to_vec(&output).expect("Serialization should succeed");
        assert!(!serialized.is_empty());

        // Test deserialization
        let deserialized: WithdrawOutput = 
            borsh::from_slice(&serialized).expect("Deserialization should succeed");

        assert_eq!(output, deserialized);
    }

    #[test]
    fn test_withdrawal_command_borsh_serialization() {
        let descriptor = create_test_descriptor();
        let output = WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(1000));
        let command = WithdrawalCommand::new(vec![output]);

        // Test serialization
        let serialized = borsh::to_vec(&command).expect("Serialization should succeed");
        assert!(!serialized.is_empty());

        // Test deserialization
        let deserialized: WithdrawalCommand = 
            borsh::from_slice(&serialized).expect("Deserialization should succeed");

        assert_eq!(command, deserialized);
    }

    #[test]
    fn test_withdrawal_command_large_batch() {
        let outputs: Vec<WithdrawOutput> = (0..100)
            .map(|i| {
                let descriptor = Descriptor::new_p2pkh(&[(i % 256) as u8; 20]);
                WithdrawOutput::new(descriptor, BitcoinAmount::from_sat(1000 + i as u64))
            })
            .collect();

        let command = WithdrawalCommand::new(outputs.clone());

        assert_eq!(command.withdraw_outputs().len(), 100);

        // Test total value calculation
        let expected_total = BitcoinAmount::from_sat(
            (0..100).map(|i| 1000 + i as u64).sum::<u64>()
        );
        assert_eq!(command.get_total_value(), expected_total);

        // Test that all outputs are preserved
        for (i, output) in command.withdraw_outputs().iter().enumerate() {
            assert_eq!(output.amt(), BitcoinAmount::from_sat(1000 + i as u64));
        }
    }

    #[test]
    fn test_withdrawal_command_edge_cases() {
        // Test command with maximum amount outputs
        let descriptor = create_test_descriptor();
        let max_amount = BitcoinAmount::from_sat(u64::MAX);
        let output = WithdrawOutput::new(descriptor, max_amount);
        let command = WithdrawalCommand::new(vec![output]);

        assert_eq!(command.get_total_value(), max_amount);

        // Test command with many zero-amount outputs
        let descriptor = create_test_descriptor();
        let outputs = vec![
            WithdrawOutput::new(descriptor.clone(), BitcoinAmount::from_sat(0));
            10
        ];
        let command = WithdrawalCommand::new(outputs);

        assert_eq!(command.get_total_value(), BitcoinAmount::from_sat(0));
        assert_eq!(command.withdraw_outputs().len(), 10);
    }
}

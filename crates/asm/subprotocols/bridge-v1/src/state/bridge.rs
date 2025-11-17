use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_txs_bridge_v1::{
    deposit::{DepositInfo, validate_deposit_output_lock, validate_drt_spending_signature},
    errors::Mismatch,
    withdrawal_fulfillment::WithdrawalFulfillmentInfo,
};
use strata_primitives::l1::{BitcoinAmount, L1BlockCommitment};

use crate::{
    errors::{DepositValidationError, WithdrawalCommandError, WithdrawalValidationError},
    state::{
        assignment::AssignmentTable,
        config::BridgeV1Config,
        deposit::{DepositEntry, DepositsTable},
        operator::OperatorTable,
        withdrawal::{OperatorClaimUnlock, WithdrawOutput, WithdrawalCommand},
    },
};

/// Main state container for the Bridge V1 subprotocol.
///
/// This structure holds all the persistent state for the bridge, including
/// operator registrations, deposit tracking, and assignment management.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    /// Table of registered bridge operators.
    operators: OperatorTable,

    /// Table of Bitcoin deposits managed by the bridge.
    deposits: DepositsTable,

    /// Table of operator assignments for withdrawal processing.
    assignments: AssignmentTable,

    /// The amount of bitcoin expected to be locked in the N/N multisig.
    denomination: BitcoinAmount,

    /// Amount the operator can take as fees for processing withdrawal.
    operator_fee: BitcoinAmount,
}

impl BridgeV1State {
    /// Creates a new bridge state with the specified configuration.
    ///
    /// Initializes all component tables as empty, creates an operator table from the provided
    /// operator public keys, and sets the expected deposit denomination and deadline duration
    /// for validation and assignment management.
    ///
    /// # Parameters
    ///
    /// - `config` - Configuration containing operator public keys, denomination, and deadline
    ///   duration
    ///
    /// # Returns
    ///
    /// A new [`BridgeV1State`] instance.
    pub fn new(config: &BridgeV1Config) -> Self {
        let operators = OperatorTable::from_operator_list(&config.operators);
        Self {
            operators,
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new(config.assignment_duration),
            denomination: config.denomination,
            operator_fee: config.operator_fee,
        }
    }

    /// Returns a reference to the operator table.
    pub fn operators(&self) -> &OperatorTable {
        &self.operators
    }

    /// Returns a reference to the deposits table.
    pub fn deposits(&self) -> &DepositsTable {
        &self.deposits
    }

    /// Returns a reference to the assignments table.
    pub fn assignments(&self) -> &AssignmentTable {
        &self.assignments
    }

    /// Validates a deposit transaction and info against bridge state requirements.
    ///
    /// This function performs comprehensive validation of a deposit by verifying:
    /// - The deposit amount matches the bridge's expected amount
    /// - The Deposit Request Transaction (DRT) spending signature is valid
    /// - The deposit output is properly locked to the aggregated operator key
    /// - The deposit index is unique within the deposits table
    ///
    /// # Parameters
    ///
    /// - `tx` - The Bitcoin transaction containing the deposit
    /// - `info` - The parsed deposit information to validate
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the deposit passes all validation checks
    /// - `Err(DepositError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount doesn't match the bridge's expected amount
    /// - The DRT spending signature is invalid or doesn't match the aggregated operator key
    /// - The deposit output lock is incorrect
    /// - A deposit with the same index already exists
    fn validate_deposit(
        &self,
        tx: &Transaction,
        info: &DepositInfo,
    ) -> Result<(), DepositValidationError> {
        // Verify the deposit amount matches the bridge's expected amount
        if info.amt.to_sat() != self.denomination.to_sat() {
            return Err(DepositValidationError::MismatchDepositAmount(Mismatch {
                expected: self.denomination.to_sat(),
                got: info.amt.to_sat(),
            }));
        }

        // Validate the DRT spending signature against the aggregated operator key
        validate_drt_spending_signature(
            tx,
            info.drt_tapscript_merkle_root,
            self.operators().agg_key(),
            info.amt.into(),
        )?;

        // Ensure the deposit output is properly locked to the aggregated operator key
        validate_deposit_output_lock(tx, self.operators().agg_key())?;

        // Verify this deposit index hasn't been used before
        if self.deposits().get_deposit(info.deposit_idx).is_some() {
            return Err(DepositValidationError::DepositIdxAlreadyExists(
                info.deposit_idx,
            ));
        }

        Ok(())
    }

    /// Processes a deposit transaction by validating and adding it to the deposits table.
    ///
    /// This function takes already parsed deposit transaction information, validates it against the
    /// current state, and creates a new deposit entry in the deposits table if
    /// validation passes. Only operators that are currently active in the N/N multisig set
    /// are included as notary operators for the deposit.
    ///
    /// # Parameters
    ///
    /// - `tx` - The deposit transaction
    /// - `info` - Parsed deposit information containing amount, destination, and outpoint
    ///
    /// # Returns
    ///
    /// - `Ok(u32)` - The deposit index assigned to the newly created deposit entry
    /// - `Err(DepositError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount is zero or negative
    /// - The internal key doesn't match the current aggregated operator key
    /// - The deposit index already exists in the deposits table
    pub fn process_deposit_tx(
        &mut self,
        tx: &Transaction,
        info: &DepositInfo,
    ) -> Result<(), DepositValidationError> {
        // Validate the deposit first
        self.validate_deposit(tx, info)?;
        let notary_operators = self.operators.current_multisig().clone();
        let entry = DepositEntry::new(info.deposit_idx, info.outpoint, notary_operators, info.amt)?;
        self.deposits.insert_deposit(entry)?;

        Ok(())
    }

    /// Adds a new withdrawal assignment to the assignments table.
    ///
    /// This retrieves the oldest unassigned deposit UTXO, validates that its amount matches
    /// the withdrawal amount, and creates a withdrawal command with the configured operator fee.
    /// The assignment is then added to the table with operators randomly selected from the
    /// currently active operators.
    ///
    /// # Parameters
    ///
    /// - `withdrawal_output` - The withdrawal output specifying destination and amount
    /// - `l1_block` - The L1 block commitment used for operator selection and deadline calculation
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal assignment was successfully added
    /// - `Err(WithdrawalCommandError)` - If no unassigned deposits, amounts mismatch, or adding new
    ///   assignment fails
    pub fn create_withdrawal_assignment(
        &mut self,
        withdrawal_output: &WithdrawOutput,
        l1_block: &L1BlockCommitment,
    ) -> Result<(), WithdrawalCommandError> {
        // Get the oldest deposit
        let deposit = self
            .deposits
            .remove_oldest_deposit()
            .ok_or(WithdrawalCommandError::NoUnassignedDeposits)?;

        if deposit.amt() != withdrawal_output.amt() {
            return Err(WithdrawalCommandError::DepositWithdrawalAmountMismatch(
                Mismatch {
                    expected: deposit.amt().to_sat(),
                    got: withdrawal_output.amt().to_sat(),
                },
            ));
        }

        let withdrawal_cmd = WithdrawalCommand::new(withdrawal_output.clone(), self.operator_fee);

        self.assignments.add_new_assignment(
            deposit,
            withdrawal_cmd,
            self.operators.current_multisig(),
            l1_block,
        )
    }

    /// Processes all expired assignments by reassigning them to new operators.
    ///
    /// This function iterates through all assignments, identifies those that have expired
    /// based on the current Bitcoin block height, and attempts to reassign them to new
    /// operators that haven't been previously assigned to the same withdrawal.
    ///
    /// # Parameters
    ///
    /// - `current_block` - The current L1 block commitment containing height and block hash
    ///
    /// # Returns
    ///
    /// - `Ok(Vec<u32>)` - Vector of deposit indices that were successfully reassigned
    /// - `Err(WithdrawalCommandError)` - If any reassignment fails
    ///
    /// # Notes
    ///
    /// If a reassignment fails for any expired assignment (e.g., no eligible operators
    /// remaining), the function returns an error and stops processing. Successfully
    /// reassigned deposits before the error are returned in the error context if needed.
    pub fn reassign_expired_assignments(
        &mut self,
        current_block: &L1BlockCommitment,
    ) -> Result<Vec<u32>, WithdrawalCommandError> {
        self.assignments
            .reassign_expired_assignments(self.operators.current_multisig(), current_block)
    }

    /// Validates the parsed withdrawal fulfillment information against assignment information.
    ///
    /// This function takes already parsed withdrawal information and validates it
    /// against the corresponding assignment entry. It checks that:
    /// - An assignment exists for the withdrawal's deposit
    /// - The operator performing the withdrawal matches the assigned operator
    /// - The withdrawal amounts and destinations match the assignment specifications
    ///
    /// # Parameters
    ///
    /// - `withdrawal_info` - Parsed withdrawal information containing operator, deposit details,
    ///   and amounts
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal is valid according to assignment information
    /// - `Err(WithdrawalValidationError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No assignment exists for the referenced deposit
    /// - The operator doesn't match the assigned operator
    /// - The withdrawal specifications don't match the assignment
    fn validate_withdrawal_fulfillment(
        &self,
        withdrawal_info: &WithdrawalFulfillmentInfo,
    ) -> Result<(), WithdrawalValidationError> {
        let deposit_idx = withdrawal_info.deposit_idx;

        // Check if an assignment exists for this deposit
        let assignment = self
            .assignments
            .get_assignment(deposit_idx)
            .ok_or(WithdrawalValidationError::NoAssignmentFound { deposit_idx })?;

        // Validate that the operator matches the assignment
        let expected_operator = assignment.current_assignee();
        let actual_operator = withdrawal_info.operator_idx;
        if expected_operator != actual_operator {
            return Err(WithdrawalValidationError::OperatorMismatch(Mismatch {
                expected: expected_operator,
                got: actual_operator,
            }));
        }

        // Validate that the deposit txid matches the assignment
        let expected_txid = assignment.deposit_txid();
        let actual_txid = withdrawal_info.deposit_txid.clone();
        if expected_txid != actual_txid {
            return Err(WithdrawalValidationError::DepositTxidMismatch(Mismatch {
                expected: expected_txid,
                got: actual_txid,
            }));
        }

        // Validate withdrawal amount against assignment command
        let expected_amount = assignment.withdrawal_command().net_amount();
        let actual_amount = withdrawal_info.withdrawal_amount;
        if expected_amount != actual_amount {
            return Err(WithdrawalValidationError::AmountMismatch(Mismatch {
                expected: expected_amount,
                got: actual_amount,
            }));
        }

        // Validate withdrawal destination against assignment command
        let expected_destination = assignment.withdrawal_command().destination().to_script();
        let actual_destination = withdrawal_info.withdrawal_destination.clone();
        if expected_destination != actual_destination {
            return Err(WithdrawalValidationError::DestinationMismatch(Mismatch {
                expected: expected_destination,
                got: actual_destination,
            }));
        }

        Ok(())
    }

    /// Processes a withdrawal fulfillment transaction by validating it, and removing the
    /// assignment from AssignmentTable.
    ///
    /// This function takes already parsed withdrawal transaction information, validates it against
    /// the current state using the assignment table, removes the assignment entry to mark the
    /// withdrawal as fulfilled. The withdrawal processing information is returned to the caller
    /// for storage in MohoState and later use by Bridge proof.
    ///
    /// # Parameters
    ///
    /// - `tx` - The withdrawal fulfillment transaction
    /// - `withdrawal_info` - Parsed withdrawal information containing operator, deposit details,
    ///   and withdrawal amounts
    ///
    /// # Returns
    ///
    /// - `Ok(WithdrawalProcessedInfo)` - The processed withdrawal information if transaction passes
    ///   validation
    /// - `Err(WithdrawalValidationError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No assignment exists for the referenced deposit
    /// - The operator doesn't match the assigned operator
    /// - The withdrawal specifications don't match the assignment
    /// - The deposit referenced in the withdrawal doesn't exist
    pub fn process_withdrawal_fulfillment_tx(
        &mut self,
        tx: &Transaction,
        withdrawal_info: &WithdrawalFulfillmentInfo,
    ) -> Result<OperatorClaimUnlock, WithdrawalValidationError> {
        self.validate_withdrawal_fulfillment(withdrawal_info)?;

        // Remove the assignment from the table to mark withdrawal as fulfilled
        // Safe to unwrap since validate_withdrawal ensures the assignment exists
        let removed_assignment = self
            .assignments
            .remove_assignment(withdrawal_info.deposit_idx)
            .expect("Assignment must exist after successful validation");

        Ok(OperatorClaimUnlock {
            fulfillment_txid: tx.compute_txid().into(),
            deposit_txid: removed_assignment.deposit_txid(),
            deposit_idx: removed_assignment.deposit_idx(),
            operator_idx: withdrawal_info.operator_idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use rand::Rng;
    use strata_asm_txs_bridge_v1::{
        deposit::DepositInfo,
        test_utils::{create_test_deposit_tx, create_test_withdrawal_fulfillment_tx},
    };
    use strata_crypto::{schnorr::EvenPublicKey, EvenSecretKey};
    use strata_primitives::{
        bitcoin_bosd::Descriptor,
        buf::Buf32,
        l1::{BitcoinAmount, L1BlockCommitment},
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::state::{
        assignment::AssignmentEntry, config::BridgeV1Config, withdrawal::WithdrawOutput,
    };

    /// Helper function to create test operator keys
    ///
    /// Creates between 2 and 5 test operators with random secret keys and converts them to the
    /// OperatorPubkeys format required by BridgeV1Config. Returns both the private
    /// keys (for signing test transactions) and public keys (for state configuration).
    ///
    /// # Returns
    ///
    /// - `Vec<EvenSecretKey>` - Private keys for creating test transactions
    /// - `Vec<EvenPublicKey>` - MuSig2 public keys for bridge configuration
    fn create_test_operators() -> (Vec<EvenSecretKey>, Vec<EvenPublicKey>) {
        let secp = Secp256k1::new();
        let mut rng = secp256k1::rand::thread_rng();
        let num_operators = rng.gen_range(2..=5);

        // Generate random operator keys
        let operators_privkeys: Vec<EvenSecretKey> = (0..num_operators)
            .map(|_| SecretKey::new(&mut rng).into())
            .collect();

        // Create operator MuSig2 public keys for config
        let operator_pubkeys: Vec<EvenPublicKey> = operators_privkeys
            .iter()
            .map(|sk| {
                let pk = PublicKey::from_secret_key(&secp, sk);
                let (xonly, _) = pk.x_only_public_key();
                let musig2_pk = Buf32::new(xonly.serialize());
                EvenPublicKey::try_from(musig2_pk).expect("valid even public key")
            })
            .collect();

        (operators_privkeys, operator_pubkeys)
    }

    fn create_test_state() -> (BridgeV1State, Vec<EvenSecretKey>) {
        let (privkeys, operators) = create_test_operators();
        let denomination = BitcoinAmount::from_sat(1_000_000);
        let config = BridgeV1Config {
            denomination,
            operators,
            assignment_duration: 144, // ~24 hours
            operator_fee: BitcoinAmount::from_sat(100_000),
        };
        let bridge_state = BridgeV1State::new(&config);
        (bridge_state, privkeys)
    }

    /// Helper function to add multiple test deposits to the bridge state.
    ///
    /// Creates the specified number of deposits with randomly generated deposit info,
    /// but ensures each deposit uses the bridge's expected denomination amount.
    /// Each deposit is processed through the full validation pipeline.
    ///
    /// # Parameters
    ///
    /// - `state` - Mutable reference to the bridge state to add deposits to
    /// - `count` - Number of deposits to create and add
    /// - `privkeys` - Private keys used to sign the deposit transactions
    fn add_deposits(
        state: &mut BridgeV1State,
        count: usize,
        privkeys: &[EvenSecretKey],
    ) -> Vec<DepositInfo> {
        let mut arb = ArbitraryGenerator::new();
        let mut infos = Vec::new();
        for _ in 0..count {
            let mut info: DepositInfo = arb.generate();
            info.amt = state.denomination;
            let tx = create_test_deposit_tx(&info, privkeys);
            state.process_deposit_tx(&tx, &info).unwrap();
            infos.push(info);
        }
        infos
    }

    /// Helper function to add deposits and immediately create withdrawal assignments.
    ///
    /// This is a convenience function that combines deposit creation with assignment
    /// creation. For each deposit added, it creates a corresponding withdrawal command
    /// and assignment. This simulates a complete deposit-to-assignment flow for testing.
    ///
    /// # Parameters
    ///
    /// - `state` - Mutable reference to the bridge state
    /// - `count` - Number of deposit-assignment pairs to create
    /// - `privkeys` - Private keys used to sign the deposit transactions
    fn add_deposits_and_assignments(
        state: &mut BridgeV1State,
        count: usize,
        privkeys: &[EvenSecretKey],
    ) {
        add_deposits(state, count, privkeys);
        let mut arb = ArbitraryGenerator::new();
        for _ in 0..count {
            let l1blk: L1BlockCommitment = arb.generate();
            let mut output: WithdrawOutput = arb.generate();
            output.amt = state.denomination;
            state.create_withdrawal_assignment(&output, &l1blk).unwrap();
        }
    }

    /// Helper function to create withdrawal info that matches an existing assignment.
    ///
    /// Extracts all the necessary information from an assignment entry to create
    /// a WithdrawalInfo struct that would pass validation. This is used in tests
    /// to create valid withdrawal fulfillment transactions.
    ///
    /// # Parameters
    ///
    /// - `assignment` - The assignment entry to extract information from
    ///
    /// # Returns
    ///
    /// A WithdrawalInfo struct with matching operator, deposit, and withdrawal details
    fn create_withdrawal_info_from_assignment(
        assignment: &AssignmentEntry,
    ) -> WithdrawalFulfillmentInfo {
        WithdrawalFulfillmentInfo {
            operator_idx: assignment.current_assignee(),
            deposit_idx: assignment.deposit_idx(),
            deposit_txid: assignment.deposit_txid(),
            withdrawal_destination: assignment.withdrawal_command().destination().to_script(),
            withdrawal_amount: assignment.withdrawal_command().net_amount(),
        }
    }

    /// Test successful deposit transaction processing.
    ///
    /// Verifies that valid deposits with correct amounts and signatures are processed
    /// successfully and stored in the deposits table with the correct information.
    #[test]
    fn test_process_deposit_tx_success() {
        let (mut bridge_state, privkeys) = create_test_state();
        for i in 0..5 {
            let mut deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
            deposit_info.amt = bridge_state.denomination;

            let deposit_tx = create_test_deposit_tx(&deposit_info, &privkeys);

            // Process the deposit
            let result = bridge_state.process_deposit_tx(&deposit_tx, &deposit_info);
            assert!(
                result.is_ok(),
                "Valid deposit should be processed successfully"
            );

            // Verify the deposit was added to the state
            assert_eq!(bridge_state.deposits().len(), i + 1);
            let stored_deposit = bridge_state
                .deposits()
                .get_deposit(deposit_info.deposit_idx)
                .unwrap();
            assert_eq!(stored_deposit.idx(), deposit_info.deposit_idx);
            assert_eq!(stored_deposit.amt(), deposit_info.amt);
            assert_eq!(stored_deposit.output(), &deposit_info.outpoint);
        }
    }

    /// Test deposit transaction rejection due to invalid amount.
    ///
    /// Verifies that deposits with amounts that don't match the bridge's expected
    /// denomination are rejected with the appropriate error type.
    #[test]
    fn test_process_deposit_tx_invalid_amount() {
        let (mut bridge_state, privkeys) = create_test_state();
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();

        let tx = create_test_deposit_tx(&deposit_info, &privkeys);

        let err = bridge_state
            .process_deposit_tx(&tx, &deposit_info)
            .unwrap_err();
        assert!(matches!(
            err,
            DepositValidationError::MismatchDepositAmount(_)
        ));
        if let DepositValidationError::MismatchDepositAmount(mismatch) = err {
            assert_eq!(mismatch.expected, bridge_state.denomination.to_sat());
            assert_eq!(mismatch.got, deposit_info.amt.to_sat());
        }

        // Verify no deposit was added
        assert_eq!(bridge_state.deposits().len(), 0);
    }

    /// Test deposit transaction rejection due to invalid signature.
    ///
    /// Verifies that deposits signed with incomplete or incorrect operator keys
    /// are rejected during signature validation.
    #[test]
    fn test_process_deposit_tx_invalid_signing_set() {
        let (mut bridge_state, mut privkeys) = create_test_state();

        let mut deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        deposit_info.amt = bridge_state.denomination;

        privkeys.pop();
        let tx = create_test_deposit_tx(&deposit_info, &privkeys);

        let err = bridge_state
            .process_deposit_tx(&tx, &deposit_info)
            .unwrap_err();

        assert!(matches!(err, DepositValidationError::DrtSignature(_)));

        // Verify no deposit was added
        assert_eq!(bridge_state.deposits().len(), 0);
    }

    /// Test successful withdrawal assignment creation.
    ///
    /// Verifies that withdrawal assignments are created correctly by consuming
    /// unassigned deposits and creating assignment entries. Tests the progression
    /// from multiple deposits to assignments until no deposits remain.
    #[test]
    fn test_create_withdrawal_assignment_success() {
        let (mut state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 4;
        add_deposits(&mut state, count, &privkeys);

        for i in 0..count {
            let unassigned_deposit_count = state.deposits.len();
            let assigned_deposit_count = state.assignments.len();
            assert_eq!(unassigned_deposit_count as usize, count - i);
            assert_eq!(assigned_deposit_count as usize, i);

            let l1blk: L1BlockCommitment = arb.generate();
            let mut output: WithdrawOutput = arb.generate();
            output.amt = state.denomination;
            let res = state.create_withdrawal_assignment(&output, &l1blk);
            assert!(res.is_ok());

            let unassigned_deposit_count = state.deposits.len();
            let assigned_deposit_count = state.assignments.len();
            assert_eq!(unassigned_deposit_count as usize, count - i - 1);
            assert_eq!(assigned_deposit_count as usize, i + 1);
        }

        let l1blk: L1BlockCommitment = arb.generate();
        let output: WithdrawOutput = arb.generate();
        let res = state.create_withdrawal_assignment(&output, &l1blk);
        assert!(res.is_err());
    }

    /// Test withdrawal assignment creation failure scenarios.
    ///
    /// Verifies that withdrawal assignment creation fails when there's a mismatch
    /// between the deposit amount and withdrawal command amount.
    #[test]
    fn test_create_withdrawal_assignment_failure() {
        let (mut state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 1;
        let deposit = add_deposits(&mut state, count, &privkeys)[0].clone();

        let l1blk: L1BlockCommitment = arb.generate();
        let output: WithdrawOutput = arb.generate();
        let err = state
            .create_withdrawal_assignment(&output, &l1blk)
            .unwrap_err();
        assert!(matches!(
            err,
            WithdrawalCommandError::DepositWithdrawalAmountMismatch(..)
        ));
        if let WithdrawalCommandError::DepositWithdrawalAmountMismatch(mismatch) = err {
            assert_eq!(mismatch.got, output.amt.to_sat());
            assert_eq!(mismatch.expected, deposit.amt.to_sat());
        }
    }

    /// Test successful withdrawal fulfillment transaction processing.
    ///
    /// Verifies that valid withdrawal fulfillment transactions that match their
    /// corresponding assignments are processed successfully and result in assignment removal.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_success() {
        let (mut bridge_state, privkeys) = create_test_state();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        for _ in 0..count {
            let assignment = bridge_state.assignments().assignments().first().unwrap();
            let withdrawal_info = create_withdrawal_info_from_assignment(assignment);
            let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
            let res = bridge_state.process_withdrawal_fulfillment_tx(&tx, &withdrawal_info);
            assert!(res.is_ok());
        }
    }

    /// Test withdrawal fulfillment rejection due to operator mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// operator performing the withdrawal doesn't match the assigned operator.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_operator_mismatch() {
        let (mut bridge_state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_operator_idx = withdrawal_info.operator_idx;
        withdrawal_info.operator_idx = arb.generate();
        let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
        let err = bridge_state
            .process_withdrawal_fulfillment_tx(&tx, &withdrawal_info)
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::OperatorMismatch(_)
        ));
        if let WithdrawalValidationError::OperatorMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_operator_idx);
            assert_eq!(mismatch.got, withdrawal_info.operator_idx);
        }
    }

    /// Test withdrawal fulfillment rejection due to deposit txid mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// referenced deposit transaction ID doesn't match the assignment.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_deposit_txid_mismatch() {
        let (mut bridge_state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_deposit_txid = withdrawal_info.deposit_txid;
        withdrawal_info.deposit_txid = arb.generate();
        let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
        let err = bridge_state
            .process_withdrawal_fulfillment_tx(&tx, &withdrawal_info)
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::DepositTxidMismatch(_)
        ));
        if let WithdrawalValidationError::DepositTxidMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_deposit_txid);
            assert_eq!(mismatch.got, withdrawal_info.deposit_txid);
        }
    }

    /// Test withdrawal fulfillment rejection due to destination mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// withdrawal destination doesn't match the destination in the assignment.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_destination_mismatch() {
        let (mut bridge_state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_withdrawal_destination = withdrawal_info.withdrawal_destination.clone();
        withdrawal_info.withdrawal_destination = arb.generate::<Descriptor>().to_script();
        let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
        let err = bridge_state
            .process_withdrawal_fulfillment_tx(&tx, &withdrawal_info)
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::DestinationMismatch(_)
        ));
        if let WithdrawalValidationError::DestinationMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_withdrawal_destination);
            assert_eq!(mismatch.got, withdrawal_info.withdrawal_destination);
        }
    }

    /// Test withdrawal fulfillment rejection due to amount mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// withdrawal amount doesn't match the amount specified in the assignment.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_amount_mismatch() {
        let (mut bridge_state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_withdrawal_amount = withdrawal_info.withdrawal_amount;
        withdrawal_info.withdrawal_amount = arb.generate();
        let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
        let err = bridge_state
            .process_withdrawal_fulfillment_tx(&tx, &withdrawal_info)
            .unwrap_err();

        assert!(matches!(err, WithdrawalValidationError::AmountMismatch(_)));
        if let WithdrawalValidationError::AmountMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_withdrawal_amount);
            assert_eq!(mismatch.got, withdrawal_info.withdrawal_amount);
        }
    }

    /// Test withdrawal fulfillment rejection when no assignment exists.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when
    /// referencing a deposit index that doesn't have a corresponding assignment.
    #[test]
    fn test_process_withdrawal_fulfillment_tx_no_assignment_found() {
        let (mut bridge_state, privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count, &privkeys);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);
        withdrawal_info.deposit_idx = arb.generate();

        let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
        let err = bridge_state
            .process_withdrawal_fulfillment_tx(&tx, &withdrawal_info)
            .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::NoAssignmentFound { .. }
        ));
        if let WithdrawalValidationError::NoAssignmentFound { deposit_idx } = err {
            assert_eq!(deposit_idx, withdrawal_info.deposit_idx);
        }
    }
}

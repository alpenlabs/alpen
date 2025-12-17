use strata_asm_txs_bridge_v1::{
    errors::Mismatch, withdrawal_fulfillment::WithdrawalFulfillmentInfo,
};

use crate::{BridgeV1State, WithdrawalValidationError};

/// Validates the parsed withdrawal fulfillment information against assignment information.
///
/// This function takes already parsed withdrawal information and validates it
/// against the corresponding assignment entry. It checks that:
/// - An assignment exists for the withdrawal's deposit
/// - The withdrawal amounts and destinations match the assignment specifications
///
/// # Parameters
///
/// - `withdrawal_info` - Parsed withdrawal information containing deposit details and amounts
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
/// - The withdrawal specifications don't match the assignment
pub(crate) fn validate_withdrawal_fulfillment_info(
    state: &BridgeV1State,
    withdrawal_info: &WithdrawalFulfillmentInfo,
) -> Result<(), WithdrawalValidationError> {
    let deposit_idx = withdrawal_info.header_aux().deposit_idx();

    // Check if an assignment exists for this deposit
    let assignment = state
        .assignments()
        .get_assignment(deposit_idx)
        .ok_or(WithdrawalValidationError::NoAssignmentFound { deposit_idx })?;

    // Validate withdrawal amount against assignment command
    let expected_amount = assignment.withdrawal_command().net_amount();
    let actual_amount = withdrawal_info.withdrawal_amount();
    if expected_amount != actual_amount {
        return Err(WithdrawalValidationError::AmountMismatch(Mismatch {
            expected: expected_amount,
            got: actual_amount,
        }));
    }

    // Validate withdrawal destination against assignment command
    let expected_destination = assignment.withdrawal_command().destination().to_script();
    let actual_destination = withdrawal_info.withdrawal_destination().clone();
    if expected_destination != actual_destination {
        return Err(WithdrawalValidationError::DestinationMismatch(Mismatch {
            expected: expected_destination,
            got: actual_destination,
        }));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_primitives::bitcoin_bosd::Descriptor;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{
        add_deposits_and_assignments, create_test_state, create_withdrawal_info_from_assignment,
    };

    /// Test successful withdrawal fulfillment transaction processing.
    ///
    /// Verifies that valid withdrawal fulfillment transactions that match their
    /// corresponding assignments are processed successfully and result in assignment removal.
    #[test]
    fn test_withdrawal_fulfillment_validation_success() {
        let (mut bridge_state, _privkeys) = create_test_state();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count);

        for _ in 0..count {
            let assignment = bridge_state.assignments().assignments().first().unwrap();
            let withdrawal_info = create_withdrawal_info_from_assignment(assignment);
            let res = validate_withdrawal_fulfillment_info(&bridge_state, &withdrawal_info);
            assert!(res.is_ok());
        }
    }

    /// Test withdrawal fulfillment rejection due to destination mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// withdrawal destination doesn't match the destination in the assignment.
    #[test]
    fn test_withdrawal_fulfillment_validation_destination_mismatch() {
        let (mut bridge_state, _privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_withdrawal_destination = withdrawal_info.withdrawal_destination().clone();
        withdrawal_info.set_withdrawal_destination(arb.generate::<Descriptor>().to_script());
        let err =
            validate_withdrawal_fulfillment_info(&bridge_state, &withdrawal_info).unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::DestinationMismatch(_)
        ));
        if let WithdrawalValidationError::DestinationMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_withdrawal_destination);
            assert_eq!(mismatch.got, *withdrawal_info.withdrawal_destination());
        }
    }

    /// Test withdrawal fulfillment rejection due to amount mismatch.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when the
    /// withdrawal amount doesn't match the amount specified in the assignment.
    #[test]
    fn test_withdrawal_fulfillment_validation_amount_mismatch() {
        let (mut bridge_state, _privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);

        let correct_withdrawal_amount = withdrawal_info.withdrawal_amount();
        withdrawal_info.set_withdrawal_amount(arb.generate());
        let err =
            validate_withdrawal_fulfillment_info(&bridge_state, &withdrawal_info).unwrap_err();

        assert!(matches!(err, WithdrawalValidationError::AmountMismatch(_)));
        if let WithdrawalValidationError::AmountMismatch(mismatch) = err {
            assert_eq!(mismatch.expected, correct_withdrawal_amount);
            assert_eq!(mismatch.got, withdrawal_info.withdrawal_amount());
        }
    }

    /// Test withdrawal fulfillment rejection when no assignment exists.
    ///
    /// Verifies that withdrawal fulfillment transactions are rejected when
    /// referencing a deposit index that doesn't have a corresponding assignment.
    #[test]
    fn test_withdrawal_fulfillment_validation_no_assignment_found() {
        let (mut bridge_state, _privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 3;
        add_deposits_and_assignments(&mut bridge_state, count);

        let assignment = bridge_state.assignments().assignments().first().unwrap();
        let mut withdrawal_info = create_withdrawal_info_from_assignment(assignment);
        withdrawal_info
            .header_aux_mut()
            .set_deposit_idx(arb.generate());

        let err =
            validate_withdrawal_fulfillment_info(&bridge_state, &withdrawal_info).unwrap_err();

        assert!(matches!(
            err,
            WithdrawalValidationError::NoAssignmentFound { .. }
        ));
        if let WithdrawalValidationError::NoAssignmentFound { deposit_idx } = err {
            assert_eq!(deposit_idx, withdrawal_info.header_aux().deposit_idx());
        }
    }
}

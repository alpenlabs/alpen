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

use alpen_reth_primitives::{WithdrawalCalldata, WithdrawalIntentEvent};
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    constants::{BRIDGEOUT_PRECOMPILE_ADDRESS, FIXED_WITHDRAWAL_WEI},
    utils::wei_to_sats,
};

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
///
/// Calldata format: `[4 bytes: selected_operator (big-endian u32)][BOSD bytes]`
/// - `u32::MAX` (`0xFFFFFFFF`): no operator selection
/// - Any other value: operator index
pub(crate) fn bridge_context_call(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let calldata = WithdrawalCalldata::decode(input.data).ok_or_else(|| {
        PrecompileError::other(
            "Calldata too short: expected at least 5 bytes (4 operator + 1 BOSD)",
        )
    })?;

    // Validate that this is a valid BOSD
    validate_bosd(&calldata.bosd)?;

    let withdrawal_amount = input.value;

    // Verify that the transaction value matches the required withdrawal amount
    if withdrawal_amount < FIXED_WITHDRAWAL_WEI {
        return Err(PrecompileError::other(
            "Invalid withdrawal value: must have 10 BTC in wei",
        ));
    }

    // Convert wei to satoshis
    let (sats, _) = wei_to_sats(withdrawal_amount);

    // Try converting sats (U256) into u64 amount
    let amount: u64 = sats.try_into().map_err(|_| {
        PrecompileError::Fatal("Withdrawal amount exceeds maximum allowed value".into())
    })?;

    // Log the bridge withdrawal intent
    let evt = WithdrawalIntentEvent {
        amount,
        destination: Bytes::from(calldata.bosd),
        selectedOperator: calldata.selected_operator.raw(),
    };

    // Create a log entry for the bridge out intent
    let logdata = LogData::from(&evt);
    input.internals.log(Log {
        address: BRIDGEOUT_PRECOMPILE_ADDRESS,
        data: logdata,
    });

    // Burn value sent to bridge by adjusting the account balance of bridge precompile
    input
        .internals
        .set_balance(BRIDGEOUT_PRECOMPILE_ADDRESS, U256::ZERO)
        .map_err(|_| {
            PrecompileError::Fatal("Failed to reset BRIDGEOUT_ADDRESS account balance".into())
        })?;

    // TODO: Properly calculate and deduct gas for the bridge out operation
    let gas_cost = 0;

    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

/// Validates that input is a valid BOSD [`Descriptor`].
fn validate_bosd(data: &[u8]) -> Result<(), PrecompileError> {
    Descriptor::from_bytes(data)
        .map_err(|_| PrecompileError::other("Invalid BOSD: expected a valid BOSD descriptor"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_bridge_types::OperatorSelection;

    use super::*;

    /// Valid P2WPKH descriptor: type tag (0x03) + 20-byte hash160.
    const VALID_P2WPKH_BOSD: &[u8; 21] = &{
        let mut buf = [0x14u8; 21];
        buf[0] = 0x03; // P2WPKH type tag
        buf
    };

    #[test]
    fn test_decode_calldata_empty() {
        assert!(WithdrawalCalldata::decode(&[]).is_none());
    }

    #[test]
    fn test_decode_calldata_no_preference() {
        let mut data = Vec::new();
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let calldata = WithdrawalCalldata::decode(&data).unwrap();
        assert_eq!(calldata.selected_operator, OperatorSelection::any());
        assert_eq!(calldata.bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_decode_calldata_operator_42() {
        let mut data = Vec::new();
        data.extend_from_slice(&42u32.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let calldata = WithdrawalCalldata::decode(&data).unwrap();
        assert_eq!(calldata.selected_operator, OperatorSelection::specific(42));
        assert_eq!(calldata.bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_decode_calldata_operator_large() {
        let idx: u32 = 0x01020304;
        let mut data = Vec::new();
        data.extend_from_slice(&idx.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let calldata = WithdrawalCalldata::decode(&data).unwrap();
        assert_eq!(calldata.selected_operator, OperatorSelection::specific(idx));
        assert_eq!(calldata.bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_decode_calldata_operator_zero() {
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let calldata = WithdrawalCalldata::decode(&data).unwrap();
        assert_eq!(calldata.selected_operator, OperatorSelection::specific(0));
        assert_eq!(calldata.bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_decode_calldata_too_short() {
        // Only 3 bytes — less than the minimum 5 (4 operator + 1 BOSD)
        let data = vec![0x00, 0x01, 0x02];
        assert!(WithdrawalCalldata::decode(&data).is_none());
    }

    #[test]
    fn test_decode_calldata_only_operator_no_bosd() {
        // Exactly 4 bytes (operator only, no BOSD)
        let data = vec![0x00, 0x00, 0x00, 0x05];
        assert!(WithdrawalCalldata::decode(&data).is_none());
    }
}

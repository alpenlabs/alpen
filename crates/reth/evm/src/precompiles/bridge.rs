use alpen_reth_primitives::{WithdrawalCalldata, WithdrawalIntentEvent};
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{constants::BRIDGEOUT_PRECOMPILE_ADDRESS, utils::wei_to_sats};

// REVIEW(STR-3676): Replace these draft values with protocol-approved launch constants.
/// Fixed raw EVM gas charged for bridge-out precompile execution.
const BRIDGEOUT_BASE_GAS: u64 = 10_000;

/// Raw EVM gas charged per calldata byte handled by the bridge-out precompile.
const BRIDGEOUT_CALLDATA_BYTE_GAS: u64 = 16;

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
///
/// Calldata format: `[4 bytes: selected_operator (big-endian u32)][BOSD bytes]`
/// - `u32::MAX` (`0xFFFFFFFF`): no operator selection
/// - Any other value: operator index
pub(crate) fn bridge_context_call(
    mut input: PrecompileInput<'_>,
    denomination_wei: U256,
    max_withdrawal_wei: Option<U256>,
) -> PrecompileResult {
    let gas_cost = bridgeout_gas_cost(input.data.len())?;
    if gas_cost > input.gas {
        return Err(PrecompileError::OutOfGas);
    }

    let calldata = WithdrawalCalldata::decode(input.data).ok_or_else(|| {
        PrecompileError::other(
            "Calldata too short: expected at least 5 bytes (4 operator + 1 BOSD)",
        )
    })?;

    // Validate that this is a valid BOSD
    validate_bosd(&calldata.bosd)?;

    let withdrawal_amount = input.value;

    // Verify that the transaction value is a positive exact multiple of the withdrawal denomination
    validate_withdrawal_amount(withdrawal_amount, denomination_wei, max_withdrawal_wei)?;

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

    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

fn bridgeout_gas_cost(calldata_len: usize) -> Result<u64, PrecompileError> {
    let calldata_len = u64::try_from(calldata_len)
        .map_err(|_| PrecompileError::Fatal("Bridgeout calldata length exceeds u64".into()))?;

    BRIDGEOUT_CALLDATA_BYTE_GAS
        .checked_mul(calldata_len)
        .and_then(|calldata_gas| BRIDGEOUT_BASE_GAS.checked_add(calldata_gas))
        .ok_or_else(|| PrecompileError::Fatal("Bridgeout gas cost overflow".into()))
}

/// Validates that the withdrawal amount is a positive exact multiple of the denomination
/// and within the optional cap.
fn validate_withdrawal_amount(
    amount: U256,
    denomination_wei: U256,
    max_withdrawal_wei: Option<U256>,
) -> Result<(), PrecompileError> {
    if amount.is_zero() || !(amount % denomination_wei).is_zero() {
        return Err(PrecompileError::other(format!(
            "Invalid withdrawal value: must be a positive exact multiple of {denomination_wei} wei",
        )));
    }
    if let Some(max) = max_withdrawal_wei {
        if amount > max {
            return Err(PrecompileError::other(format!(
                "Withdrawal value {amount} exceeds maximum allowed {max} wei",
            )));
        }
    }
    Ok(())
}

/// Validates that input is a valid BOSD [`Descriptor`].
fn validate_bosd(data: &[u8]) -> Result<(), PrecompileError> {
    Descriptor::from_bytes(data)
        .map_err(|_| PrecompileError::other("Invalid BOSD: expected a valid BOSD descriptor"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_ol_bridge_types::OperatorSelection;

    use super::*;
    use crate::utils::{u256_from, WEI_PER_BTC};

    /// Test-only denomination constant (1 BTC in wei).
    const FIXED_WITHDRAWAL_WEI: U256 = u256_from(WEI_PER_BTC);

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

    #[test]
    fn test_bridgeout_gas_cost_includes_base_and_calldata_bytes() {
        let calldata_len = 4 + VALID_P2WPKH_BOSD.len();

        assert_eq!(
            bridgeout_gas_cost(calldata_len).unwrap(),
            BRIDGEOUT_BASE_GAS + BRIDGEOUT_CALLDATA_BYTE_GAS * calldata_len as u64
        );
    }

    #[test]
    fn test_bridgeout_gas_cost_scales_with_calldata_len() {
        let short = bridgeout_gas_cost(5).unwrap();
        let long = bridgeout_gas_cost(6).unwrap();

        assert_eq!(long - short, BRIDGEOUT_CALLDATA_BYTE_GAS);
    }

    #[test]
    fn test_bridgeout_gas_cost_rejects_overflow() {
        assert!(bridgeout_gas_cost(usize::MAX).is_err());
    }

    // --- withdrawal amount validation tests ---

    fn max_withdrawal() -> Option<U256> {
        Some(FIXED_WITHDRAWAL_WEI * U256::from(10))
    }

    #[test]
    fn test_validate_withdrawal_exact_denomination() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI,
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_ok());
    }

    #[test]
    fn test_validate_withdrawal_exact_multiple() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI * U256::from(3),
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_ok());
    }

    #[test]
    fn test_validate_withdrawal_zero_rejected() {
        assert!(
            validate_withdrawal_amount(U256::ZERO, FIXED_WITHDRAWAL_WEI, max_withdrawal()).is_err()
        );
    }

    #[test]
    fn test_validate_withdrawal_non_multiple_rejected() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI + U256::from(1),
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_err());
    }

    #[test]
    fn test_validate_withdrawal_below_denomination_rejected() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI - U256::from(1),
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_err());
    }

    #[test]
    fn test_validate_withdrawal_exceeds_cap() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI * U256::from(11),
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_err());
    }

    #[test]
    fn test_validate_withdrawal_at_cap() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI * U256::from(10),
            FIXED_WITHDRAWAL_WEI,
            max_withdrawal()
        )
        .is_ok());
    }

    #[test]
    fn test_validate_withdrawal_no_cap() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI * U256::from(100),
            FIXED_WITHDRAWAL_WEI,
            None
        )
        .is_ok());
    }
}

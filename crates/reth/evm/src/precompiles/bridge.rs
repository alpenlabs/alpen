use alpen_reth_primitives::{WithdrawalCalldata, WithdrawalIntentEvent};
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_bridge_params::BridgeParams;
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
    bridge_params: BridgeParams,
) -> PrecompileResult {
    if !input.is_direct_call() {
        return Err(PrecompileError::other(
            "bridgeout precompile must be invoked via CALL",
        ));
    }

    let gas_cost = bridgeout_gas_cost(input.data.len())?;
    if gas_cost > input.gas {
        return Err(PrecompileError::OutOfGas);
    }

    let calldata = WithdrawalCalldata::decode(input.data).ok_or_else(|| {
        PrecompileError::other(
            "Calldata too short: expected at least 5 bytes (4 operator + 1 BOSD)",
        )
    })?;

    // Validate that this is a valid BOSD.
    validate_bosd(&calldata.bosd, &bridge_params)?;

    // Verify that the transaction value is a positive exact multiple of the withdrawal
    // denomination.
    let amount = validate_withdrawal_amount(input.value, &bridge_params)?;

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

fn sats_to_withdrawal_amount(sats: U256) -> Result<u64, PrecompileError> {
    sats.try_into()
        .map_err(|_| PrecompileError::other("Withdrawal amount exceeds maximum allowed value"))
}

fn bridgeout_gas_cost(calldata_len: usize) -> Result<u64, PrecompileError> {
    let calldata_len = u64::try_from(calldata_len)
        .map_err(|_| PrecompileError::Fatal("Bridgeout calldata length exceeds u64".into()))?;

    BRIDGEOUT_CALLDATA_BYTE_GAS
        .checked_mul(calldata_len)
        .and_then(|calldata_gas| BRIDGEOUT_BASE_GAS.checked_add(calldata_gas))
        .ok_or_else(|| PrecompileError::Fatal("Bridgeout gas cost overflow".into()))
}

/// Validates that the withdrawal amount is a positive exact multiple of the denomination and cap.
fn validate_withdrawal_amount(
    amount_wei: U256,
    bridge_params: &BridgeParams,
) -> Result<u64, PrecompileError> {
    let (amount_sats, remainder_wei) = wei_to_sats(amount_wei);
    if !remainder_wei.is_zero() {
        return Err(PrecompileError::other(format!(
            "Invalid withdrawal value {amount_wei}: must be an exact number of satoshis",
        )));
    }

    let amount_sats = sats_to_withdrawal_amount(amount_sats)?;

    if !bridge_params.validate_withdrawal_amount(amount_sats) {
        return Err(PrecompileError::other(format!(
            "Invalid withdrawal value: {amount_sats} sats must be a positive exact multiple of {} sats and within {:?} sats",
            bridge_params.denomination(),
            bridge_params.max_withdrawal_amount()
        )));
    }

    Ok(amount_sats)
}

/// Validates that input is a valid BOSD [`Descriptor`] within the configured limit.
fn validate_bosd(data: &[u8], bridge_params: &BridgeParams) -> Result<(), PrecompileError> {
    if !bridge_params.validate_withdrawal_descriptor_len(data.len()) {
        return Err(PrecompileError::other(format!(
            "Invalid BOSD: descriptor length {} exceeds maximum {}",
            data.len(),
            bridge_params.max_withdrawal_descriptor_len()
        )));
    }

    Descriptor::from_bytes(data)
        .map_err(|_| PrecompileError::other("Invalid BOSD: expected a valid BOSD descriptor"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use reth_evm::EvmInternals;
    use revm::{
        context::{BlockEnv, Journal, JournalEntry, JournalTr},
        database::EmptyDB,
        primitives::address,
    };
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
    const MAX_DESCRIPTOR_LEN: u32 = 81;

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

    #[test]
    fn test_sats_to_withdrawal_amount_rejects_overflow_as_recoverable_error() {
        let err = sats_to_withdrawal_amount(U256::from(u64::MAX) + U256::from(1)).unwrap_err();

        assert!(matches!(err, PrecompileError::Other(_)));
    }

    // --- withdrawal amount validation tests ---

    fn bridge_params() -> BridgeParams {
        BridgeParams::default()
    }

    fn bridge_params_without_cap() -> BridgeParams {
        BridgeParams::new(100_000_000, None).unwrap()
    }

    fn valid_bridgeout_calldata() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);
        data
    }

    #[test]
    fn test_bridgeout_rejects_delegatecall_apparent_value() {
        let calldata = valid_bridgeout_calldata();
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller: address!("1111111111111111111111111111111111111111"),
            value: FIXED_WITHDRAWAL_WEI,
            target_address: address!("2222222222222222222222222222222222222222"),
            bytecode_address: BRIDGEOUT_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        let error = bridge_context_call(input, bridge_params()).unwrap_err();

        assert_eq!(
            error,
            PrecompileError::Other("bridgeout precompile must be invoked via CALL".into())
        );
    }

    #[test]
    fn test_bridgeout_accepts_direct_call_value() {
        let calldata = valid_bridgeout_calldata();
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller: address!("1111111111111111111111111111111111111111"),
            value: FIXED_WITHDRAWAL_WEI,
            target_address: BRIDGEOUT_PRECOMPILE_ADDRESS,
            bytecode_address: BRIDGEOUT_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        assert!(bridge_context_call(input, bridge_params()).is_ok());
    }

    #[test]
    fn test_validate_withdrawal_exact_denomination() {
        assert_eq!(
            validate_withdrawal_amount(FIXED_WITHDRAWAL_WEI, &bridge_params()).unwrap(),
            100_000_000
        );
    }

    #[test]
    fn test_validate_withdrawal_exact_multiple() {
        assert_eq!(
            validate_withdrawal_amount(FIXED_WITHDRAWAL_WEI * U256::from(3), &bridge_params())
                .unwrap(),
            300_000_000
        );
    }

    #[test]
    fn test_validate_withdrawal_zero_rejected() {
        assert!(validate_withdrawal_amount(U256::ZERO, &bridge_params()).is_err());
    }

    #[test]
    fn test_validate_withdrawal_non_multiple_rejected() {
        assert!(
            validate_withdrawal_amount(FIXED_WITHDRAWAL_WEI + U256::from(1), &bridge_params())
                .is_err()
        );
    }

    #[test]
    fn test_validate_withdrawal_below_denomination_rejected() {
        assert!(
            validate_withdrawal_amount(FIXED_WITHDRAWAL_WEI - U256::from(1), &bridge_params())
                .is_err()
        );
    }

    #[test]
    fn test_validate_withdrawal_exceeds_cap() {
        assert!(validate_withdrawal_amount(
            FIXED_WITHDRAWAL_WEI * U256::from(11),
            &bridge_params()
        )
        .is_err());
    }

    #[test]
    fn test_validate_withdrawal_at_cap() {
        assert_eq!(
            validate_withdrawal_amount(FIXED_WITHDRAWAL_WEI * U256::from(10), &bridge_params())
                .unwrap(),
            1_000_000_000
        );
    }

    #[test]
    fn test_validate_withdrawal_no_cap() {
        assert_eq!(
            validate_withdrawal_amount(
                FIXED_WITHDRAWAL_WEI * U256::from(100),
                &bridge_params_without_cap()
            )
            .unwrap(),
            10_000_000_000
        );
    }

    #[test]
    fn test_validate_bosd_accepts_descriptor_at_limit() {
        let mut bosd = vec![0u8; MAX_DESCRIPTOR_LEN as usize];
        bosd[0] = 0x00;

        assert!(validate_bosd(&bosd, &bridge_params()).is_ok());
    }

    #[test]
    fn test_validate_bosd_rejects_oversized_descriptor() {
        let mut bosd = vec![0u8; MAX_DESCRIPTOR_LEN as usize + 1];
        bosd[0] = 0x00;

        assert!(validate_bosd(&bosd, &bridge_params()).is_err());
    }

    #[test]
    fn test_validate_bosd_rejects_malformed_descriptor() {
        let bosd = [0x03, 0x01, 0x02, 0x03];

        assert!(validate_bosd(&bosd, &bridge_params()).is_err());
    }
}

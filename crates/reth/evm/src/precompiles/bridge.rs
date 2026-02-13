use alpen_reth_primitives::WithdrawalIntentEvent;

/// Sentinel value indicating no operator was selected for withdrawal assignment.
const NO_SELECTED_OPERATOR: u32 = u32::MAX;
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    constants::{BRIDGEOUT_PRECOMPILE_ADDRESS, FIXED_WITHDRAWAL_WEI},
    utils::wei_to_sats,
};

/// Maximum number of bytes used to encode the operator index in calldata.
/// Operator index is a u32, so at most 4 bytes.
const MAX_OPERATOR_INDEX_LEN: usize = 4;

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
///
/// Calldata format: `[1 byte B][B bytes: operator index (big-endian)][BOSD bytes]`
/// - B=0: no operator preference
/// - B=1..4: operator index encoded as B big-endian bytes
/// - B>4: invalid
pub(crate) fn bridge_context_call(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let (preferred_operator, bosd_data) = parse_calldata(input.data)?;

    // Validate that this is a valid BOSD
    let _ = try_into_bosd(bosd_data)?;

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
        destination: Bytes::from(bosd_data.to_vec()),
        preferredOperator: preferred_operator,
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

/// Parses bridge out calldata into a preferred operator index and BOSD bytes.
///
/// Format: `[1 byte B][B bytes: operator index (big-endian)][BOSD bytes]`
/// - B=0: no selection, returns [`NO_SELECTED_OPERATOR`]
/// - B=1..4: decodes B bytes as a big-endian u32 operator index
/// - B>4: error
fn parse_calldata(data: &[u8]) -> Result<(u32, &[u8]), PrecompileError> {
    let (&b, rest) = data
        .split_first()
        .ok_or_else(|| PrecompileError::other("Empty calldata"))?;

    let b = b as usize;

    if b == 0 {
        return Ok((NO_SELECTED_OPERATOR, rest));
    }

    if b > MAX_OPERATOR_INDEX_LEN {
        return Err(PrecompileError::other(
            "Invalid operator index length: exceeds maximum of 4 bytes",
        ));
    }

    if rest.len() < b {
        return Err(PrecompileError::other(
            "Calldata too short for operator index",
        ));
    }

    let (operator_bytes, bosd_data) = rest.split_at(b);

    let operator_idx = operator_bytes
        .iter()
        .fold(0u32, |acc, &byte| (acc << 8) | byte as u32);

    Ok((operator_idx, bosd_data))
}

/// Ensures that input is a valid BOSD [`Descriptor`].
fn try_into_bosd(maybe_bosd: &[u8]) -> Result<Descriptor, PrecompileError> {
    let desc = Descriptor::from_bytes(maybe_bosd);
    match desc {
        Ok(valid_desc) => Ok(valid_desc),
        Err(_) => Err(PrecompileError::other(
            "Invalid BOSD: expected a valid BOSD descriptor",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMMY_BOSD: &[u8] = &[0x00; 20]; // placeholder BOSD bytes

    #[test]
    fn test_parse_calldata_empty() {
        let result = parse_calldata(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_calldata_no_preference() {
        let mut data = vec![0x00];
        data.extend_from_slice(DUMMY_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, NO_SELECTED_OPERATOR);
        assert_eq!(bosd, DUMMY_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_1_byte() {
        let mut data = vec![1, 42];
        data.extend_from_slice(DUMMY_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, 42);
        assert_eq!(bosd, DUMMY_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_4_bytes() {
        let idx: u32 = 0x01020304;
        let mut data = vec![4];
        data.extend_from_slice(&idx.to_be_bytes());
        data.extend_from_slice(DUMMY_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, idx);
        assert_eq!(bosd, DUMMY_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_zero_4_bytes() {
        let mut data = vec![4, 0, 0, 0, 0];
        data.extend_from_slice(DUMMY_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, 0);
        assert_eq!(bosd, DUMMY_BOSD);
    }

    #[test]
    fn test_parse_calldata_b_too_large() {
        let data = vec![5, 0, 0, 0, 0, 0];
        let result = parse_calldata(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_calldata_truncated_operator_bytes() {
        // B=4 but only 2 operator bytes follow
        let data = vec![4, 0x01, 0x02];
        let result = parse_calldata(&data);
        assert!(result.is_err());
    }
}

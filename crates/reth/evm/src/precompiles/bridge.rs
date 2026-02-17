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
/// - B=0: no operator selection
/// - B=1..4: operator index encoded as B big-endian bytes
/// - B>4: invalid
pub(crate) fn bridge_context_call(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let (selected_operator, bosd_data) = parse_calldata(input.data)?;

    // Validate that this is a valid BOSD
    validate_bosd(bosd_data)?;

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
        selectedOperator: selected_operator,
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

/// Parses bridge out calldata into a selected operator index and BOSD bytes.
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

    if bosd_data.is_empty() {
        return Err(PrecompileError::other(
            "Calldata missing BOSD data after operator index",
        ));
    }

    let operator_idx = operator_bytes
        .iter()
        .fold(0u32, |acc, &byte| (acc << 8) | byte as u32);

    Ok((operator_idx, bosd_data))
}

/// Validates that input is a valid BOSD [`Descriptor`].
fn validate_bosd(data: &[u8]) -> Result<(), PrecompileError> {
    Descriptor::from_bytes(data)
        .map_err(|_| PrecompileError::other("Invalid BOSD: expected a valid BOSD descriptor"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valid P2WPKH descriptor: type tag (0x03) + 20-byte hash160.
    const VALID_P2WPKH_BOSD: &[u8; 21] = &{
        let mut buf = [0x14u8; 21];
        buf[0] = 0x03; // P2WPKH type tag
        buf
    };

    #[test]
    fn test_parse_calldata_empty() {
        let result = parse_calldata(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_calldata_no_preference() {
        let mut data = vec![0x00];
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, NO_SELECTED_OPERATOR);
        assert_eq!(bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_1_byte() {
        let mut data = vec![1, 42];
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, 42);
        assert_eq!(bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_4_bytes() {
        let idx: u32 = 0x01020304;
        let mut data = vec![4];
        data.extend_from_slice(&idx.to_be_bytes());
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, idx);
        assert_eq!(bosd, VALID_P2WPKH_BOSD);
    }

    #[test]
    fn test_parse_calldata_operator_zero_4_bytes() {
        let mut data = vec![4, 0, 0, 0, 0];
        data.extend_from_slice(VALID_P2WPKH_BOSD);

        let (operator, bosd) = parse_calldata(&data).unwrap();
        assert_eq!(operator, 0);
        assert_eq!(bosd, VALID_P2WPKH_BOSD);
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

    #[test]
    fn test_parse_calldata_empty_bosd_after_operator() {
        // B=1 with operator byte but no BOSD data following
        let data = vec![1, 0x05];
        let result = parse_calldata(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_calldata_mismatched_b_eats_bosd_bytes() {
        // Encoder intended B=1 with operator=3, but mistakenly set B=2.
        // parse_calldata trusts B, so the BOSD type tag (0x03) is consumed
        // as part of the operator index, producing a wrong operator and shifted BOSD.
        let mut data = vec![2, 0x03]; // B=2, first operator byte
        data.extend_from_slice(VALID_P2WPKH_BOSD); // type tag (0x03) eaten as 2nd operator byte

        let (operator, bosd) = parse_calldata(&data).unwrap();
        // Operator becomes (0x03 << 8) | 0x03 = 771 instead of intended 3
        assert_eq!(operator, 0x0303);
        // BOSD is truncated by 1 byte — no longer a valid descriptor
        assert_eq!(bosd.len(), VALID_P2WPKH_BOSD.len() - 1);
        // The downstream validate_bosd catches the corruption
        assert!(validate_bosd(bosd).is_err());
    }
}

use alpen_reth_primitives::WithdrawalIntentEvent;
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    constants::{BRIDGEOUT_ADDRESS, FIXED_WITHDRAWAL_WEI},
    utils::wei_to_sats,
};

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
pub(crate) fn bridge_context_call(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let destination = input.data;

    // Validate that this is a valid BOSD
    let _ = try_into_bosd(destination)?;

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
        destination: Bytes::from(destination.to_vec()),
    };

    let _logdata = LogData::from(&evt);
    // input.internals.db_mut()..log(Log {
    //     address: BRIDGEOUT_ADDRESS,
    //     data: logdata,
    // });

    let mut account = input
        .internals
        .load_account(BRIDGEOUT_ADDRESS) // Error case should never occur
        .map_err(|_| PrecompileError::Fatal("Failed to load BRIDGEOUT_ADDRESS account".into()))?;

    // Burn value sent to bridge by adjusting the account balance of bridge precompile
    account.info.balance = U256::ZERO;

    // TODO: Properly calculate and deduct gas for the bridge out operation
    let gas_cost = 0;

    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

/// Ensure that input is a valid BOSD [`Descriptor`].
fn try_into_bosd(maybe_bosd: &[u8]) -> Result<Descriptor, PrecompileError> {
    let desc = Descriptor::from_bytes(maybe_bosd);
    match desc {
        Ok(valid_desc) => Ok(valid_desc),
        Err(_) => Err(PrecompileError::other(
            "Invalid BOSD: expected a valid BOSD descriptor",
        )),
    }
}

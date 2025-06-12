use alpen_reth_primitives::WithdrawalIntentEvent;
use revm::{
    context::{ContextTr, JournalTr, Transaction},
    precompile::{PrecompileError, PrecompileOutput, PrecompileResult},
};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    constants::{BRIDGEOUT_ADDRESS, FIXED_WITHDRAWAL_WEI},
    utils::wei_to_sats,
};

/// Ensure that input is a valid BOSD [`Descriptor`].
fn try_into_bosd(maybe_bosd: &Bytes) -> Result<Descriptor, PrecompileError> {
    let desc = Descriptor::from_bytes(maybe_bosd.as_ref());
    match desc {
        Ok(valid_desc) => Ok(valid_desc),
        Err(_) => Err(PrecompileError::other(
            "Invalid BOSD: expected a valid BOSD descriptor",
        )),
    }
}

/// Placeholder for the `bridgeout` precompile; required for the precompile registry.
pub(crate) fn bridgeout_precompile(_input: &Bytes, _gas_limit: u64) -> PrecompileResult {
    let gas_cost = 0;
    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
pub(crate) fn bridge_context_call<CTX>(
    destination: &Bytes,
    _gas_limit: u64,
    evmctx: &mut CTX,
) -> PrecompileResult
where
    CTX: ContextTr,
{
    // Validate that this is a valid BOSD
    let _ = try_into_bosd(destination)?;

    let withdrawal_amount = evmctx.tx().value();

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
        destination: destination.clone(),
    };
    let logdata = LogData::from(&evt);

    evmctx.journal().log(Log {
        address: BRIDGEOUT_ADDRESS,
        data: logdata,
    });

    let mut account = evmctx
        .journal()
        .load_account(BRIDGEOUT_ADDRESS) // Error case should never occur
        .map_err(|_| PrecompileError::Fatal("Failed to load BRIDGEOUT_ADDRESS account".into()))?;

    // Burn value sent to bridge by adjusting the account balance of bridge precompile
    account.info.balance = U256::ZERO;

    // TODO: Properly calculate and deduct gas for the bridge out operation
    let gas_cost = 0;

    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

use alpen_reth_primitives::WithdrawalIntentEvent;
use revm::{
    primitives::{PrecompileError, PrecompileErrors, PrecompileOutput, PrecompileResult},
    ContextStatefulPrecompile, Database,
};
use revm_primitives::{Bytes, Log, LogData, U256};
use strata_primitives::bitcoin_bosd::Descriptor;

pub(crate) use crate::constants::BRIDGEOUT_ADDRESS;
use crate::utils::wei_to_sats;

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

/// Custom precompile to burn rollup native token and add bridge out intent of equal amount.
/// Bridge out intent is created during block payload generation.
/// This precompile validates transaction and burns the bridge out amount.
#[derive(Debug)]
pub(crate) struct BridgeoutPrecompile {
    fixed_withdrawal_wei: U256,
}

impl BridgeoutPrecompile {
    pub(crate) const fn new(fixed_withdrawal_wei: U256) -> Self {
        Self {
            fixed_withdrawal_wei,
        }
    }
}

impl<DB: Database> ContextStatefulPrecompile<DB> for BridgeoutPrecompile {
    fn call(
        &self,
        destination: &Bytes,
        _gas_limit: u64,
        evmctx: &mut revm::InnerEvmContext<DB>,
    ) -> PrecompileResult {
        // Validate that this is a valid BOSD
        let _ = try_into_bosd(destination)?;

        let withdrawal_amount = evmctx
            .balance(BRIDGEOUT_ADDRESS)
            .map_err(|_| PrecompileError::other("Could not read balance"))?
            .data;

        // Verify that the transaction value matches the required withdrawal amount
        if withdrawal_amount < self.fixed_withdrawal_wei {
            return Err(PrecompileError::other(
                "Invalid withdrawal value: must have 10 BTC in wei",
            )
            .into());
        }

        // Convert wei to satoshis
        let (sats, _) = wei_to_sats(withdrawal_amount);

        // Try converting sats (U256) into u64 amount
        let amount: u64 = sats.try_into().map_err(|_| PrecompileErrors::Fatal {
            msg: "Withdrawal amount exceeds maximum allowed value".into(),
        })?;

        // Log the bridge withdrawal intent
        let evt = WithdrawalIntentEvent {
            amount,
            // PERF: This may be improved by avoiding the allocation.
            destination: destination.clone(),
        };
        let logdata = LogData::from(&evt);

        evmctx.journaled_state.log(Log {
            address: BRIDGEOUT_ADDRESS,
            data: logdata,
        });

        // Burn value sent to bridge by adjusting the account balance of bridge precompile
        let mut account = evmctx
            .load_account(BRIDGEOUT_ADDRESS)
            // Error case should never occur
            .map_err(|_| PrecompileErrors::Fatal {
                msg: "Failed to load BRIDGEOUT_ADDRESS account".into(),
            })?;

        account.info.balance = U256::ZERO;

        // TODO: Properly calculate and deduct gas for the bridge out operation
        let gas_cost = 0;

        Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
    }
}

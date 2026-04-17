use std::{str::FromStr, time::Duration};

use alloy::{
    network::TransactionBuilder, primitives::U256, providers::Provider,
    rpc::types::TransactionInput,
};
use alpen_reth_primitives::WithdrawalCalldata;
use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{Address, Amount},
    KeychainKind,
};
use indicatif::ProgressBar;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_ol_bridge_types::OperatorSelection;
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    alpen::AlpenWallet,
    constants::SATS_TO_WEI,
    link::{OnchainObject, PrettyPrint},
    seed::Seed,
    settings::Settings,
    signet::SignetWallet,
};

/// Withdraws BTC from Alpen to signet. The amount must be a positive
/// multiple of the bridge denomination configured in params.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "withdraw")]
pub struct WithdrawArgs {
    /// the signet address to send funds to. defaults to a new internal wallet address
    #[argh(positional)]
    address: Option<String>,

    /// amount to withdraw in sats (must be a positive multiple of the denomination).
    /// defaults to one denomination unit.
    #[argh(option)]
    amount: Option<u64>,

    /// selected operator index for withdrawal assignment
    #[argh(option)]
    operator: Option<u32>,
}

pub async fn withdraw(
    args: WithdrawArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let address = args
        .address
        .map(|a| {
            let unchecked = Address::from_str(&a).user_error(format!(
                "Invalid signet address: '{a}'. Must be a valid Bitcoin address."
            ))?;
            let checked = unchecked
                .require_network(settings.params.network)
                .user_error(format!(
                    "Provided address '{a}' is not valid for network '{}'",
                    settings.params.network
                ))?;
            Ok(checked)
        })
        .transpose()?;

    let mut l1w = SignetWallet::new(
        &seed,
        settings.params.network,
        settings.signet_backend.clone(),
    )
    .internal_error("Failed to load signet wallet")?;
    l1w.sync()
        .await
        .internal_error("Failed to sync signet wallet")?;
    let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
        .user_error("Invalid Alpen endpoint URL. Check the configuration")?;

    let address = match address {
        Some(a) => a,
        None => {
            let info = l1w.reveal_next_address(KeychainKind::External);
            l1w.persist()
                .internal_error("Failed to persist signet wallet")?;
            info.address
        }
    };

    let denomination = settings.params.deposit_amount;
    let bridge_out_amount = resolve_withdrawal_amount(args.amount, denomination)
        .map_err(|msg| DisplayedError::UserError(msg, Box::new(())))?;
    println!("Bridging out {} to {address}", bridge_out_amount);

    let bosd: Descriptor = address
        .try_into()
        .user_error("Failed to convert address to BOSD descriptor")?;

    let selected_operator = match args.operator {
        Some(idx) => OperatorSelection::specific(idx),
        None => OperatorSelection::any(),
    };
    let calldata = WithdrawalCalldata {
        selected_operator,
        bosd: bosd.to_bytes(),
    }
    .encode();

    let tx = l2w
        .transaction_request()
        .with_to(settings.bridge_alpen_address)
        .with_value(U256::from(bridge_out_amount.to_sat() as u128 * SATS_TO_WEI))
        .input(TransactionInput::new(calldata.into()));

    let pb = ProgressBar::new_spinner().with_message("Broadcasting transaction");
    pb.enable_steady_tick(Duration::from_millis(100));
    let res = l2w
        .send_transaction(tx)
        .await
        .internal_error("Failed to broadcast Alpen transaction")?;
    pb.finish_with_message("Broadcast successful");
    println!(
        "{}",
        OnchainObject::from(res.tx_hash())
            .with_maybe_explorer(settings.blockscout_endpoint.as_deref())
            .pretty(),
    );

    Ok(())
}

/// Resolves the withdrawal amount from an optional user-provided value and the denomination.
///
/// Returns the validated amount, or an error message if the amount is invalid.
fn resolve_withdrawal_amount(
    amount_sats: Option<u64>,
    denomination: Amount,
) -> Result<Amount, String> {
    match amount_sats {
        Some(sats) => {
            if sats == 0 || !sats.is_multiple_of(denomination.to_sat()) {
                return Err(format!(
                    "Amount must be a positive multiple of {denomination}"
                ));
            }
            Ok(Amount::from_sat(sats))
        }
        None => Ok(denomination),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENOM: Amount = Amount::from_sat(100_000_000); // 1 BTC

    #[test]
    fn test_none_defaults_to_denomination() {
        let result = resolve_withdrawal_amount(None, DENOM).unwrap();
        assert_eq!(result, DENOM);
    }

    #[test]
    fn test_exact_denomination_accepted() {
        let result = resolve_withdrawal_amount(Some(100_000_000), DENOM).unwrap();
        assert_eq!(result, Amount::from_sat(100_000_000));
    }

    #[test]
    fn test_exact_multiple_accepted() {
        let result = resolve_withdrawal_amount(Some(300_000_000), DENOM).unwrap();
        assert_eq!(result, Amount::from_sat(300_000_000));
    }

    #[test]
    fn test_zero_rejected() {
        assert!(resolve_withdrawal_amount(Some(0), DENOM).is_err());
    }

    #[test]
    fn test_non_multiple_rejected() {
        assert!(resolve_withdrawal_amount(Some(150_000_000), DENOM).is_err());
    }

    #[test]
    fn test_below_denomination_rejected() {
        assert!(resolve_withdrawal_amount(Some(50_000_000), DENOM).is_err());
    }
}

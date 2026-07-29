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
use strata_bridge_params::BridgeParams;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_ol_bridge_types::OperatorSelection;
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    alpen::AlpenWallet,
    constants::SATS_TO_WEI,
    ee::EePreset,
    link::{OnchainObject, PrettyPrint},
    seed::Seed,
    settings::Settings,
    signet::SignetWallet,
};

/// Withdraws BTC from Alpen to signet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "withdraw")]
pub struct WithdrawArgs {
    /// source Alpen EE account preset: "ALPN" (uses alpen_endpoint) or "NPAL"
    /// (uses nepal_endpoint). defaults to "ALPN" when omitted.
    #[argh(positional)]
    preset: Option<EePreset>,

    /// the signet address to send funds to. defaults to a new internal wallet address
    #[argh(option)]
    address: Option<String>,

    /// amount to withdraw in sats (must be a positive multiple of the denomination).
    /// defaults to one denomination unit.
    #[argh(option)]
    amount: Option<u64>,

    /// selected operator index for withdrawal assignment
    #[argh(option)]
    operator: Option<u32>,
}

/// Resolves the Alpen EE RPC endpoint for the selected preset.
///
/// `ALPN` (or no preset) uses `alpen_endpoint`; `NPAL` uses `nepal_endpoint`,
/// erroring if it is not configured.
pub(crate) fn resolve_endpoint<'a>(
    preset: Option<EePreset>,
    alpen_endpoint: &'a str,
    nepal_endpoint: Option<&'a str>,
) -> Result<&'a str, DisplayedError> {
    match preset.unwrap_or(EePreset::Alpn) {
        EePreset::Alpn => Ok(alpen_endpoint),
        EePreset::Npal => nepal_endpoint.ok_or_else(|| {
            DisplayedError::UserError(
                "NPAL preset selected but 'nepal_endpoint' is not set in the config file"
                    .to_string(),
                Box::new(()),
            )
        }),
    }
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
                .require_network(settings.network)
                .user_error(format!(
                    "Provided address '{a}' is not valid for network '{}'",
                    settings.network
                ))?;
            Ok(checked)
        })
        .transpose()?;

    let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
        .internal_error("Failed to load signet wallet")?;
    l1w.sync()
        .await
        .internal_error("Failed to sync signet wallet")?;
    let endpoint = resolve_endpoint(
        args.preset,
        &settings.alpen_endpoint,
        settings.nepal_endpoint.as_deref(),
    )?;
    let l2w = AlpenWallet::new(&seed, endpoint)
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

    let bridge_out_amount = resolve_withdrawal_amount(args.amount, &settings.bridge_params)?;
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

/// Resolves the withdrawal amount from an optional user-provided value.
///
/// Defaults to one denomination if no amount is provided.
fn resolve_withdrawal_amount(
    amount_sats: Option<u64>,
    bridge_params: &BridgeParams,
) -> Result<Amount, DisplayedError> {
    let sats = amount_sats.unwrap_or(bridge_params.denomination());
    if !bridge_params.validate_withdrawal_amount(sats) {
        let denom = bridge_params.denomination();
        let mut msg = format!("Amount must be a positive multiple of {denom} sats");
        if let Some(max) = bridge_params.max_withdrawal_amount() {
            msg.push_str(&format!(" and at most {max} sats"));
        }
        return Err(DisplayedError::UserError(msg, Box::new(())));
    }
    Ok(Amount::from_sat(sats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> BridgeParams {
        BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap()
    }

    const ALPN_URL: &str = "https://alpn.example";
    const NPAL_URL: &str = "https://npal.example";

    #[test]
    fn endpoint_defaults_to_alpen() {
        let got = resolve_endpoint(None, ALPN_URL, Some(NPAL_URL)).unwrap();
        assert_eq!(got, ALPN_URL);
    }

    #[test]
    fn endpoint_alpn_uses_alpen() {
        let got = resolve_endpoint(Some(EePreset::Alpn), ALPN_URL, Some(NPAL_URL)).unwrap();
        assert_eq!(got, ALPN_URL);
    }

    #[test]
    fn endpoint_npal_uses_nepal() {
        let got = resolve_endpoint(Some(EePreset::Npal), ALPN_URL, Some(NPAL_URL)).unwrap();
        assert_eq!(got, NPAL_URL);
    }

    #[test]
    fn endpoint_npal_without_config_errors() {
        assert!(resolve_endpoint(Some(EePreset::Npal), ALPN_URL, None).is_err());
    }

    #[test]
    fn endpoint_alpn_without_nepal_ok() {
        let got = resolve_endpoint(Some(EePreset::Alpn), ALPN_URL, None).unwrap();
        assert_eq!(got, ALPN_URL);
    }

    #[test]
    fn test_none_defaults_to_denomination() {
        let result = resolve_withdrawal_amount(None, &params()).unwrap();
        assert_eq!(result, Amount::from_sat(100_000_000));
    }

    #[test]
    fn test_exact_denomination_accepted() {
        let result = resolve_withdrawal_amount(Some(100_000_000), &params()).unwrap();
        assert_eq!(result, Amount::from_sat(100_000_000));
    }

    #[test]
    fn test_exact_multiple_accepted() {
        let result = resolve_withdrawal_amount(Some(300_000_000), &params()).unwrap();
        assert_eq!(result, Amount::from_sat(300_000_000));
    }

    #[test]
    fn test_zero_rejected() {
        assert!(resolve_withdrawal_amount(Some(0), &params()).is_err());
    }

    #[test]
    fn test_non_multiple_rejected() {
        assert!(resolve_withdrawal_amount(Some(150_000_000), &params()).is_err());
    }

    #[test]
    fn test_below_denomination_rejected() {
        assert!(resolve_withdrawal_amount(Some(50_000_000), &params()).is_err());
    }

    #[test]
    fn test_exceeds_cap_rejected() {
        assert!(resolve_withdrawal_amount(Some(1_100_000_000), &params()).is_err());
    }

    #[test]
    fn test_at_cap_accepted() {
        let result = resolve_withdrawal_amount(Some(1_000_000_000), &params()).unwrap();
        assert_eq!(result, Amount::from_sat(1_000_000_000));
    }
}

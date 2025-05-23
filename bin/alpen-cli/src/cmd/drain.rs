use std::str::FromStr;

use alloy::{
    primitives::{Address as AlpenAddress, U256},
    providers::{Provider, WalletProvider},
};
use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{Address, Amount},
    coin_selection::InsufficientFunds,
    error::CreateTxError,
};
use colored::Colorize;

use crate::{
    alpen::AlpenWallet,
    constants::SATS_TO_WEI,
    errors::{DisplayableError, DisplayedError},
    link::{OnchainObject, PrettyPrint},
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, SignetWallet},
};

/// Drains the internal wallet to the provided
/// signet and Alpen addresses
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "drain")]
pub struct DrainArgs {
    /// a signet address for signet funds to be drained to
    #[argh(option, short = 's')]
    signet_address: Option<String>,

    /// an Alpen address for Alpen funds to be drained to
    #[argh(option, short = 'r')]
    alpen_address: Option<String>,

    /// override signet fee rate in sat/vbyte. must be >=1
    #[argh(option)]
    fee_rate: Option<u64>,
}

/// Target address not provided
#[derive(Debug, Clone, Copy)]
pub struct MissingTargetAddress;

pub async fn drain(
    DrainArgs {
        signet_address,
        alpen_address,
        fee_rate,
    }: DrainArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    if alpen_address.is_none() && signet_address.is_none() {
        return Err(DisplayedError::UserError(
            "Missing target address. Must provide a `signet` address or `alpen` address.".into(),
            Box::new(MissingTargetAddress),
        ));
    }

    let signet_address = signet_address
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

    let alpen_address = alpen_address
        .map(|a| {
            AlpenAddress::from_str(&a).user_error(format!(
                "Invalid Alpen address '{a}'. Must be an EVM-compatible address"
            ))
        })
        .transpose()?;

    if let Some(address) = signet_address {
        let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
            .internal_error("Failed to load signet wallet")?;
        l1w.sync()
            .await
            .internal_error("Failed to sync signet wallet")?;
        let balance = l1w.balance();
        if balance.untrusted_pending > Amount::ZERO {
            println!(
                "{}",
                "You have pending funds on signet that won't be included in the drain".yellow()
            );
        }
        let fee_rate = get_fee_rate(fee_rate, settings.signet_backend.as_ref()).await;
        log_fee_rate(&fee_rate);

        let mut psbt = {
            let mut builder = l1w.build_tx();
            builder.drain_wallet();
            builder.drain_to(address.script_pubkey());
            builder.fee_rate(fee_rate);
            match builder.finish() {
                Ok(psbt) => psbt,
                Err(CreateTxError::CoinSelection(e @ InsufficientFunds { .. })) => {
                    return Err(DisplayedError::UserError(
                        "Failed to create PSBT".to_string(),
                        Box::new(e),
                    ));
                }
                Err(e) => panic!("Unexpected error in creating PSBT: {e:?}"),
            }
        };
        l1w.sign(&mut psbt, Default::default())
            .expect("tx should be signed");
        let tx = psbt.extract_tx().expect("tx should be signed and ready");
        settings
            .signet_backend
            .broadcast_tx(&tx)
            .await
            .internal_error("Failed to broadcast signet transaction")?;
        let txid = tx.compute_txid();
        println!(
            "{}",
            OnchainObject::from(&txid)
                .with_maybe_explorer(settings.mempool_space_endpoint.as_deref())
                .pretty()
        );
        println!("Drained signet wallet to {address}",);
    }

    if let Some(address) = alpen_address {
        let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
            .user_error("Invalid Alpen endpoint URL. Check the config file")?;
        let balance = l2w
            .get_balance(l2w.default_signer_address())
            .await
            .internal_error("Failed to fetch Alpen balance")?;
        if balance == U256::ZERO {
            println!("No Alpen bitcoin to send");
        }

        let estimate_tx = l2w
            .transaction_request()
            .from(l2w.default_signer_address())
            .to(address)
            .value(U256::from(1));

        let gas_price = l2w
            .get_gas_price()
            .await
            .internal_error("Failed to fetch Alpen gas price.")? as u64;
        let gas_estimate = l2w
            .estimate_gas(estimate_tx)
            .await
            .internal_error("Failed to estimate Alpen gas")?;

        let total_fee = gas_estimate * gas_price;
        let max_send_amount = balance.saturating_sub(U256::from(total_fee));

        let tx = l2w.transaction_request().to(address).value(max_send_amount);

        let res = l2w
            .send_transaction(tx)
            .await
            .internal_error("Failed to broadcast strata transaction")?;

        println!(
            "{}",
            OnchainObject::from(res.tx_hash())
                .with_maybe_explorer(settings.blockscout_endpoint.as_deref())
                .pretty()
        );

        println!(
            "Drained {} from Alpen wallet to {}",
            Amount::from_sat((max_send_amount / U256::from(SATS_TO_WEI)).wrapping_to()),
            address,
        );
    }

    Ok(())
}

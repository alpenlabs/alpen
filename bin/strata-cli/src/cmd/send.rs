use std::str::FromStr;

use alloy::{
    network::TransactionBuilder,
    primitives::{Address as StrataAddress, U256},
    providers::Provider,
    rpc::types::TransactionRequest,
};
use argh::FromArgs;
use bdk_wallet::bitcoin::{Address, Amount};

use crate::{
    constants::SATS_TO_WEI,
    errors::{DisplayableError, DisplayedError},
    link::{OnchainObject, PrettyPrint},
    net_type::NetworkType,
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, SignetWallet},
    strata::StrataWallet,
};

/// Send some bitcoin from the internal wallet.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "send")]
pub struct SendArgs {
    /// either "signet" or "strata"
    #[argh(positional)]
    network_type: String,

    /// amount to send in sats
    #[argh(positional)]
    amount: u64,

    /// address to send to
    #[argh(positional)]
    address: String,

    /// override signet fee rate in sat/vbyte. must be >=1
    #[argh(option)]
    fee_rate: Option<u64>,
}

pub async fn send(args: SendArgs, seed: Seed, settings: Settings) -> Result<(), DisplayedError> {
    let network_type = args
        .network_type
        .parse()
        .user_error("invalid network type")?;

    match network_type {
        NetworkType::Signet => {
            let amount = Amount::from_sat(args.amount);
            let address = Address::from_str(&args.address)
                .user_error(format!(
                    "Invalid signet address: '{}'. Must be a valid Bitcoin address.",
                    args.address
                ))?
                .require_network(settings.network)
                .user_error(format!(
                    "Wrong network '{}' for address: '{}'.",
                    settings.network, args.address
                ))?;
            let mut l1w =
                SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
                    .internal_error("Failed to load signet wallet")?;
            l1w.sync()
                .await
                .internal_error("Failed to sync signet wallet")?;
            let fee_rate = get_fee_rate(args.fee_rate, settings.signet_backend.as_ref()).await;
            log_fee_rate(&fee_rate);
            let mut psbt = {
                let mut builder = l1w.build_tx();
                builder.add_recipient(address.script_pubkey(), amount);
                builder.fee_rate(fee_rate);
                builder
                    .finish()
                    .internal_error("Failed to build signet transaction")?
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
                    .pretty(),
            );
        }
        NetworkType::Strata => {
            let l2w = StrataWallet::new(&seed, &settings.strata_endpoint)?;
            let address = StrataAddress::from_str(&args.address).user_error(format!(
                "Invalid strata address '{}'. Must be an EVM-compatible address.",
                args.address
            ))?;
            let tx = TransactionRequest::default()
                .with_to(address)
                .with_value(U256::from(args.amount as u128 * SATS_TO_WEI));
            let res = l2w
                .send_transaction(tx)
                .await
                .internal_error("Failed to broadcast strata transaction")?;
            println!(
                "{}",
                OnchainObject::from(res.tx_hash())
                    .with_maybe_explorer(settings.blockscout_endpoint.as_deref())
                    .pretty(),
            );
        }
    };

    println!("Sent {} to {}", Amount::from_sat(args.amount), args.address,);
    Ok(())
}

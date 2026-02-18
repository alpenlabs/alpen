use std::{str::FromStr, time::Duration};

use alloy::{
    network::TransactionBuilder, primitives::U256, providers::Provider,
    rpc::types::TransactionInput,
};
use argh::FromArgs;
use bdk_wallet::{bitcoin::Address, KeychainKind};
use indicatif::ProgressBar;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{
    alpen::AlpenWallet,
    constants::SATS_TO_WEI,
    link::{OnchainObject, PrettyPrint},
    seed::Seed,
    settings::Settings,
    signet::SignetWallet,
};

/// Withdraws 10 BTC from Alpen to signet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "withdraw")]
pub struct WithdrawArgs {
    /// the signet address to send funds to. defaults to a new internal wallet address
    #[argh(positional)]
    address: Option<String>,

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

    let bridge_out_amount = settings.params.deposit_amount;
    println!("Bridging out {} to {address}", bridge_out_amount);

    let bosd: Descriptor = address
        .try_into()
        .user_error("Failed to convert address to BOSD descriptor")?;

    let calldata = encode_bridge_out_calldata(args.operator, &bosd);

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

/// Encodes bridge out calldata: `[4 bytes: selected_operator (big-endian u32)][BOSD bytes]`.
///
/// When no operator is selected, `u32::MAX` is written as the sentinel value.
fn encode_bridge_out_calldata(operator: Option<u32>, bosd: &Descriptor) -> Vec<u8> {
    let bosd_bytes = bosd.to_bytes();
    let mut buf = Vec::with_capacity(4 + bosd_bytes.len());
    let selected = operator.unwrap_or(u32::MAX);
    buf.extend_from_slice(&selected.to_be_bytes());
    buf.extend_from_slice(&bosd_bytes);
    buf
}

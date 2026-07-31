use alloy::{
    primitives::U256,
    providers::{Provider, WalletProvider},
};
use argh::FromArgs;
use bdk_wallet::bitcoin::Amount;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    alpen::AlpenWallet, cmd::withdraw::resolve_endpoint, constants::SATS_TO_WEI, ee::EePreset,
    net_type::NetworkType, seed::Seed, settings::Settings, signet::SignetWallet,
};

/// Prints the wallet's current balance(s)
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "balance")]
pub struct BalanceArgs {
    /// either "signet" or "alpen"
    #[argh(positional)]
    network_type: String,

    /// for "alpen": which EE to query, "ALPN" (uses alpen_endpoint) or "NPAL"
    /// (uses nepal_endpoint). defaults to "ALPN". ignored for "signet".
    #[argh(positional)]
    preset: Option<EePreset>,
}

pub async fn balance(
    args: BalanceArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let network_type = args
        .network_type
        .parse()
        .user_error(format!("Invalid network type '{}'", args.network_type))?;

    if let NetworkType::Signet = network_type {
        let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
            .internal_error("Failed to load signet wallet")?;

        l1w.sync()
            .await
            .internal_error("Failed to sync signet wallet")?;

        let balance = l1w.balance();
        println!("Total: {}", balance.total());
        println!("  Confirmed: {}", balance.confirmed);
        println!("  Trusted pending: {}", balance.trusted_pending);
        println!("  Untrusted pending: {}", balance.untrusted_pending);
        println!("  Immature: {}", balance.immature);
    }

    if let NetworkType::Alpen = network_type {
        let endpoint = resolve_endpoint(
            args.preset,
            &settings.alpen_endpoint,
            settings.nepal_endpoint.as_deref(),
        )?;
        let l2w = AlpenWallet::new(&seed, endpoint)
            .user_error("Invalid Alpen endpoint URL. Check the config file")?;
        println!("Getting balance...");
        let eth_balance = l2w
            .get_balance(l2w.default_signer_address())
            .await
            .internal_error("Failed to fetch Alpen balance")?;
        let sats = (eth_balance / U256::from(SATS_TO_WEI))
            .try_into()
            .expect("to fit into u64");
        let balance = Amount::from_sat(sats);

        println!("\nTotal: {balance}");
    }
    Ok(())
}

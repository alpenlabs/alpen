use alloy::providers::WalletProvider;
use argh::FromArgs;
use bdk_wallet::KeychainKind;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    alpen::AlpenWallet, net_type::NetworkType, seed::Seed, settings::Settings, signet::SignetWallet,
};

/// Prints a new address for the internal wallet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "receive")]
pub struct ReceiveArgs {
    /// either "signet" or "alpen"
    #[argh(positional)]
    network_type: String,
}

pub async fn receive(
    args: ReceiveArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let network_type = args
        .network_type
        .parse()
        .user_error(format!("invalid network type '{}'", args.network_type))?;

    let address = match network_type {
        NetworkType::Signet => {
            let mut l1w =
                SignetWallet::new(&seed, settings.params.network, settings.signet_backend.clone())
                    .internal_error("Failed to load signet wallet")?;

            println!("Syncing signet wallet...");
            l1w.sync()
                .await
                .internal_error("Failed to sync signet wallet")?;
            println!("Wallet synced.");

            let address_info = l1w.reveal_next_address(KeychainKind::External);

            l1w.persist()
                .internal_error("Failed to persist signet wallet")?;

            address_info.address.to_string()
        }
        NetworkType::Alpen => {
            let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
                .user_error("Invalid Alpen endpoint URL. Check the config file")?;
            l2w.default_signer_address().to_string()
        }
    };

    println!("{address}");
    Ok(())
}

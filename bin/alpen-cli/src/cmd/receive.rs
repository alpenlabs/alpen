use alloy::providers::WalletProvider;
use argh::FromArgs;
use bdk_wallet::KeychainKind;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    alpen::AlpenWallet, bitcoin::BitcoinWallet, chain::Chain, seed::Seed, settings::Settings,
};

/// Prints a new address for the internal wallet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "receive")]
pub struct ReceiveArgs {
    /// either "bitcoin" or "alpen"
    #[argh(positional)]
    chain: String,
}

pub async fn receive(
    args: ReceiveArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let chain = args
        .chain
        .parse()
        .user_error(format!("invalid chain '{}'", args.chain))?;

    let address = match chain {
        Chain::Bitcoin => {
            let mut l1w =
                BitcoinWallet::new(&seed, settings.network, settings.bitcoin_backend.clone())
                    .internal_error("Failed to load Bitcoin wallet")?;

            println!("Syncing Bitcoin wallet...");
            l1w.sync()
                .await
                .internal_error("Failed to sync Bitcoin wallet")?;
            println!("Wallet synced.");

            let address_info = l1w.reveal_next_address(KeychainKind::External);

            l1w.persist()
                .internal_error("Failed to persist Bitcoin wallet")?;

            address_info.address.to_string()
        }
        Chain::Alpen => {
            let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
                .user_error("Invalid Alpen endpoint URL. Check the config file")?;
            l2w.default_signer_address().to_string()
        }
    };

    println!("{address}");
    Ok(())
}

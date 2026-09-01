use alloy::{
    primitives::U256,
    providers::{Provider, WalletProvider},
};
use argh::FromArgs;
use bdk_wallet::bitcoin::Amount;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    alpen::AlpenWallet, bitcoin::BitcoinWallet, chain::Chain, constants::SATS_TO_WEI, seed::Seed,
    settings::Settings,
};

/// Prints the wallet's current balance(s)
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "balance")]
pub struct BalanceArgs {
    /// either "bitcoin" or "alpen"
    #[argh(positional)]
    chain: String,
}

pub async fn balance(
    args: BalanceArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let chain = args
        .chain
        .parse()
        .user_error(format!("Invalid chain '{}'", args.chain))?;

    if let Chain::Bitcoin = chain {
        let mut l1w = BitcoinWallet::new(&seed, settings.network, settings.bitcoin_backend.clone())
            .internal_error("Failed to load Bitcoin wallet")?;

        l1w.sync()
            .await
            .internal_error("Failed to sync Bitcoin wallet")?;

        let balance = l1w.balance();
        println!("Total: {}", balance.total());
        println!("  Confirmed: {}", balance.confirmed);
        println!("  Trusted pending: {}", balance.trusted_pending);
        println!("  Untrusted pending: {}", balance.untrusted_pending);
        println!("  Immature: {}", balance.immature);
    }

    if let Chain::Alpen = chain {
        let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
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

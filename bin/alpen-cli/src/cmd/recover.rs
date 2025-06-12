use argh::FromArgs;
use bdk_wallet::{
    bitcoin::Amount, chain::ChainOracle, coin_selection::InsufficientFunds,
    descriptor::IntoWalletDescriptor, error::CreateTxError, KeychainKind, Wallet,
};
use chrono::Utc;
use colored::Colorize;

use crate::{
    constants::RECOVERY_DESC_CLEANUP_DELAY,
    errors::{DisplayableError, DisplayedError},
    recovery::DescriptorRecovery,
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, sync_wallet, SignetWallet},
};

/// Attempts a recovery of old deposit transactions
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "recover")]
pub struct RecoverArgs {
    /// override signet fee rate in sat/vbyte. must be >=1
    #[argh(option)]
    fee_rate: Option<u64>,
}

pub async fn recover(
    args: RecoverArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
        .internal_error("Failed to load signet wallet")?;
    l1w.sync()
        .await
        .internal_error("Failed to sync signet wallet")?;

    println!("Opening descriptor recovery");
    let mut descriptor_file = DescriptorRecovery::open(&seed, &settings.descriptor_db)
        .await
        .internal_error("Failed to open descriptor recovery file")?;
    let current_height = l1w
        .local_chain()
        .get_chain_tip()
        .expect("valid chain tip")
        .height;

    println!("Current signet chain height: {current_height}");
    let descs = descriptor_file
        .read_descs_after_block(current_height)
        .await
        .internal_error("Failed to read descriptors after chain height")?;

    if descs.is_empty() {
        println!("Nothing to recover");
        return Ok(());
    }

    let fee_rate = get_fee_rate(args.fee_rate, settings.signet_backend.as_ref()).await;
    log_fee_rate(&fee_rate);

    for (key, desc) in descs {
        let desc = desc
            .clone()
            .into_wallet_descriptor(l1w.secp_ctx(), settings.network)
            .internal_error("Failed to convert to wallet descriptor")?;

        let mut recovery_wallet = Wallet::create_single(desc)
            .network(settings.network)
            .create_wallet_no_persist()
            .internal_error("Failed to create recovery wallet")?;

        // reveal the address for the wallet so we can sync it
        let address = recovery_wallet.reveal_next_address(KeychainKind::External);
        sync_wallet(&mut recovery_wallet, settings.signet_backend.clone())
            .await
            .internal_error("Failed to sync recovery wallet")?;
        let needs_recovery = recovery_wallet.balance().confirmed > Amount::ZERO;

        if !needs_recovery {
            assert!(key.len() > 4);
            let desc_height = u32::from_be_bytes(unsafe { *(key[..4].as_ptr() as *const [_; 4]) });
            if desc_height + RECOVERY_DESC_CLEANUP_DELAY > current_height {
                descriptor_file
                    .remove(key)
                    .internal_error("Failed to remove old descriptor")?;
                println!(
                    "removed old, already claimed descriptor due for recovery at {desc_height}"
                );
            }
            continue;
        }

        recovery_wallet.transactions().for_each(|tx| {
            l1w.apply_unconfirmed_txs([(tx.tx_node.tx, Utc::now().timestamp() as u64)]);
        });

        let recover_to = l1w.reveal_next_address(KeychainKind::External).address;
        println!(
            "Recovering a deposit transaction from address {} to {}",
            address.to_string().yellow(),
            recover_to.to_string().yellow()
        );

        // we want to drain the recovery path to the l1 wallet
        let mut psbt = {
            let mut builder = recovery_wallet.build_tx();
            builder.drain_to(recover_to.script_pubkey());
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

        recovery_wallet
            .sign(&mut psbt, Default::default())
            .expect("tx should be signed");

        let tx = psbt.extract_tx().expect("tx should be signed and ready");
        settings
            .signet_backend
            .broadcast_tx(&tx)
            .await
            .internal_error("Failed to broadcast signet transaction")?
    }

    Ok(())
}

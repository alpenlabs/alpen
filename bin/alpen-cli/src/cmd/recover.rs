use std::collections::{BTreeMap, HashSet};

use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{secp256k1::SECP256K1, Amount, FeeRate, PrivateKey, ScriptBuf},
    chain::ChainOracle,
    coin_selection::InsufficientFunds,
    descriptor::IntoWalletDescriptor,
    error::CreateTxError,
    KeychainKind, Wallet,
};
use chrono::Utc;
use colored::Colorize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_primitives::crypto::even_kp;

use crate::{
    cmd::deposit::{bridge_in_descriptor, compute_recover_at_height},
    constants::{RECOVERY_DESC_CLEANUP_DELAY, SEED_RECOVERY_GAP_LIMIT},
    link::{OnchainObject, PrettyPrint},
    recovery::DescriptorRecovery,
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, sync_wallet, SignetWallet},
};

/// Attempts a recovery of old deposit transactions
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "recover")]
pub struct RecoverArgs {
    /// override signet fee rate in sat/vbyte; the effective rate is at least 1
    #[argh(option)]
    fee_rate: Option<u64>,
}

/// Returns whether an already-claimed descriptor's cleanup grace window has elapsed.
fn cleanup_delay_elapsed(recover_at: u32, current_height: u32) -> bool {
    current_height >= recover_at.saturating_add(RECOVERY_DESC_CLEANUP_DELAY)
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
        .read_descs(..=current_height)
        .await
        .internal_error("Failed to read descriptors after chain height")?;

    if descs.is_empty() {
        println!("No descriptors in the local database");
    }

    let fee_rate = get_fee_rate(args.fee_rate, settings.signet_backend.as_ref()).await;
    log_fee_rate(&fee_rate);

    let mut drained_recovery_scripts = HashSet::new();
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
        let address = recovery_wallet
            .reveal_next_address(KeychainKind::External)
            .address;
        sync_wallet(&mut recovery_wallet, settings.signet_backend.clone())
            .await
            .internal_error("Failed to sync recovery wallet")?;
        let needs_recovery = recovery_wallet.balance().confirmed > Amount::ZERO;

        if !needs_recovery {
            if cleanup_delay_elapsed(key.recover_at, current_height) {
                descriptor_file
                    .remove(&key)
                    .internal_error("Failed to remove old descriptor")?;
                println!(
                    "removed old, already claimed descriptor due for recovery at {}",
                    key.recover_at
                );
            }
            continue;
        }

        println!(
            "Recovering a deposit transaction from recovery address {}",
            address.to_string().yellow(),
        );
        drain_recovery_path(&mut recovery_wallet, &mut l1w, &settings, fee_rate).await?;
        drained_recovery_scripts.insert(address.script_pubkey());
    }

    let highest_discovered_counter = recover_from_seed(
        &seed,
        &settings,
        &mut l1w,
        fee_rate,
        &drained_recovery_scripts,
    )
    .await?;
    descriptor_file
        .ensure_reclaim_counter_at_least(highest_discovered_counter.unwrap_or(0))
        .await
        .internal_error("Failed to save the reconstructed reclaim counter")?;

    Ok(())
}

/// Drains `recovery_wallet`'s reclaim path (policy path index 1: recovery pubkey + timelock,
/// see [`bridge_in_descriptor`]) to `l1w`, signing and broadcasting the spend.
async fn drain_recovery_path(
    recovery_wallet: &mut Wallet,
    l1w: &mut Wallet,
    settings: &Settings,
    fee_rate: FeeRate,
) -> Result<(), DisplayedError> {
    recovery_wallet.transactions().for_each(|tx| {
        l1w.apply_unconfirmed_txs([(tx.tx_node.tx, Utc::now().timestamp() as u64)]);
    });

    let recover_to = l1w.reveal_next_address(KeychainKind::External).address;
    println!(
        "Recovering to wallet address {}",
        recover_to.to_string().yellow()
    );

    let policy = recovery_wallet
        .policies(KeychainKind::External)
        .expect("valid descriptor use")
        .expect("a policy");

    // we want to drain the recovery path to the l1 wallet
    let mut psbt = {
        let mut builder = recovery_wallet.build_tx();
        // we want to spend via the 2nd option - the recovery + delay
        builder.policy_path(
            BTreeMap::from([(policy.id, vec![1])]),
            KeychainKind::External,
        );
        builder.drain_wallet();
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

    assert!(
        recovery_wallet
            .sign(&mut psbt, Default::default())
            .expect("sign to be ok"),
        "transaction should be finalized"
    );

    let tx = psbt.extract_tx().expect("tx should be signed and ready");
    settings
        .signet_backend
        .broadcast_tx(&tx)
        .await
        .internal_error("Failed to broadcast signet transaction")?;

    println!(
        "{}",
        OnchainObject::from(&tx.compute_txid())
            .with_maybe_explorer(settings.mempool_space_endpoint.as_deref())
            .pretty()
    );

    Ok(())
}

#[derive(Debug)]
struct SeedRecoveryCandidate {
    counter: u32,
    script_pubkey: ScriptBuf,
}

fn seed_recovery_wallet(
    seed: &Seed,
    settings: &Settings,
    counter: u32,
) -> Result<Wallet, DisplayedError> {
    let reclaim_keypair = seed.drt_reclaim_keypair(counter);
    let (secret_key, _) = even_kp((reclaim_keypair.secret_key, reclaim_keypair.public_key));
    let recovery_private_key = PrivateKey::new(secret_key.into(), settings.network);
    let descriptor = bridge_in_descriptor(
        settings.bridge_musig2_pubkey,
        recovery_private_key,
        settings.recovery_delay,
    );
    let wallet_descriptor = descriptor
        .into_wallet_descriptor(SECP256K1, settings.network)
        .internal_error("Failed to convert to wallet descriptor")?;

    Wallet::create_single(wallet_descriptor)
        .network(settings.network)
        .create_wallet_no_persist()
        .internal_error("Failed to create recovery wallet")
}

/// Finds reclaim-key counters with on-chain history in gap-limit-sized backend scans.
async fn discover_seed_candidates(
    seed: &Seed,
    settings: &Settings,
    known_used_scripts: &HashSet<ScriptBuf>,
) -> Result<Vec<SeedRecoveryCandidate>, DisplayedError> {
    let mut discovered = Vec::new();
    let mut batch_start = 0u32;
    let scan_checkpoint = seed_recovery_wallet(seed, settings, 0)?.latest_checkpoint();

    loop {
        let batch_end = batch_start
            .checked_add(SEED_RECOVERY_GAP_LIMIT)
            .expect("reclaim-key scan range must fit in u32");
        let mut candidates = Vec::with_capacity(SEED_RECOVERY_GAP_LIMIT as usize);
        let mut scripts_to_scan = Vec::with_capacity(SEED_RECOVERY_GAP_LIMIT as usize);

        for counter in batch_start..batch_end {
            let mut wallet = seed_recovery_wallet(seed, settings, counter)?;
            let script_pubkey = wallet
                .reveal_next_address(KeychainKind::External)
                .address
                .script_pubkey();
            if !known_used_scripts.contains(&script_pubkey) {
                scripts_to_scan.push(script_pubkey.clone());
            }
            candidates.push(SeedRecoveryCandidate {
                counter,
                script_pubkey,
            });
        }

        let backend_used_scripts = if scripts_to_scan.is_empty() {
            HashSet::new()
        } else {
            settings
                .signet_backend
                .scan_scripts(scripts_to_scan, scan_checkpoint.clone())
                .await
                .internal_error("Failed to scan seed recovery scripts")?
        };
        let mut discovered_in_batch = candidates
            .into_iter()
            .filter(|candidate| {
                known_used_scripts.contains(&candidate.script_pubkey)
                    || backend_used_scripts.contains(&candidate.script_pubkey)
            })
            .collect::<Vec<_>>();

        if discovered_in_batch.is_empty() {
            break;
        }

        discovered.append(&mut discovered_in_batch);
        batch_start = batch_end;
    }

    Ok(discovered)
}

/// Reconstructs the allocator high-water mark without spending any recovered outputs.
pub(crate) async fn reconstruct_reclaim_counter(
    seed: &Seed,
    settings: &Settings,
) -> Result<u32, DisplayedError> {
    let candidates = discover_seed_candidates(seed, settings, &HashSet::new()).await?;
    Ok(candidates
        .last()
        .map(|candidate| candidate.counter)
        .unwrap_or(0))
}

/// Reconstructs and recovers deposits directly from the seed, for deposits whose descriptor DB
/// entry is missing. Candidate scripts are scanned in batches so Bitcoin Core only replays the
/// chain once per gap-limit window, rather than once per candidate. Descriptors use the network's
/// *current* bridge pubkey and recovery delay; if either changed since a deposit was created, that
/// deposit won't be found here.
async fn recover_from_seed(
    seed: &Seed,
    settings: &Settings,
    l1w: &mut Wallet,
    fee_rate: FeeRate,
    drained_recovery_scripts: &HashSet<ScriptBuf>,
) -> Result<Option<u32>, DisplayedError> {
    println!("Scanning for deposits reconstructable from the seed alone...");

    let candidates = discover_seed_candidates(seed, settings, drained_recovery_scripts).await?;
    let highest_discovered_counter = candidates.last().map(|candidate| candidate.counter);
    let mut found_any = false;
    for candidate in candidates {
        if drained_recovery_scripts.contains(&candidate.script_pubkey) {
            continue;
        }

        let counter = candidate.counter;
        let mut recovery_wallet = seed_recovery_wallet(seed, settings, counter)?;
        let address = recovery_wallet
            .reveal_next_address(KeychainKind::External)
            .address;
        debug_assert_eq!(address.script_pubkey(), candidate.script_pubkey);

        sync_wallet(&mut recovery_wallet, settings.signet_backend.clone())
            .await
            .internal_error("Failed to sync recovery wallet")?;

        if recovery_wallet.transactions().next().is_none() {
            continue;
        }

        let current_height = recovery_wallet
            .local_chain()
            .get_chain_tip()
            .expect("valid chain tip")
            .height;
        let matured = recovery_wallet
            .list_unspent()
            .filter_map(|utxo| utxo.chain_position.confirmation_height_upper_bound())
            .all(|confirmed_at| {
                current_height
                    >= compute_recover_at_height(
                        confirmed_at,
                        settings.recovery_delay as u32,
                        settings.finality_depth,
                    )
            });

        if matured && recovery_wallet.balance().confirmed > Amount::ZERO {
            found_any = true;
            println!(
                "Recovering a deposit transaction (counter {counter}) from recovery address {}",
                address.to_string().yellow(),
            );
            drain_recovery_path(&mut recovery_wallet, l1w, settings, fee_rate).await?;
        }
    }

    if !found_any {
        println!("Nothing found to recover from the seed.");
    }

    Ok(highest_discovered_counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_delay_not_elapsed_keeps_descriptor() {
        let recover_at = 1_000;

        assert!(!cleanup_delay_elapsed(recover_at, recover_at));
        assert!(!cleanup_delay_elapsed(
            recover_at,
            recover_at + RECOVERY_DESC_CLEANUP_DELAY - 1
        ));
    }

    #[test]
    fn test_cleanup_delay_exactly_elapsed_removes_descriptor() {
        let recover_at = 1_000;

        assert!(cleanup_delay_elapsed(
            recover_at,
            recover_at + RECOVERY_DESC_CLEANUP_DELAY
        ));
    }

    #[test]
    fn test_cleanup_delay_well_past_removes_descriptor() {
        let recover_at = 1_000;

        assert!(cleanup_delay_elapsed(
            recover_at,
            recover_at + RECOVERY_DESC_CLEANUP_DELAY + 1_000
        ));
    }

    #[test]
    fn test_cleanup_delay_saturates_near_max_height() {
        let recover_at = u32::MAX;

        assert!(cleanup_delay_elapsed(recover_at, u32::MAX));
        assert!(!cleanup_delay_elapsed(recover_at, u32::MAX - 1));
    }
}

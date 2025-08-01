use std::{str::FromStr, time::Duration};

use alloy::{primitives::Address as AlpenAddress, providers::WalletProvider};
use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{script::PushBytesBuf, secp256k1::SECP256K1, PrivateKey, XOnlyPublicKey},
    chain::ChainOracle,
    coin_selection::InsufficientFunds,
    descriptor::IntoWalletDescriptor,
    error::CreateTxError,
    template::DescriptorTemplateOut,
    KeychainKind, TxOrdering, Wallet,
};
use colored::Colorize;
use indicatif::ProgressBar;
use rand_core::OsRng;
use shrex::encode;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_primitives::crypto::even_kp;

use crate::{
    alpen::AlpenWallet,
    constants::SIGNET_BLOCK_TIME,
    link::{OnchainObject, PrettyPrint},
    recovery::DescriptorRecovery,
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, SignetWallet},
};

/// Deposits 10 BTC from signet into Alpen
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "deposit")]
pub struct DepositArgs {
    /// the Alpen address to deposit the funds into. defaults to the
    /// wallet's internal address.
    #[argh(positional)]
    alpen_address: Option<String>,

    /// override signet fee rate in sat/vbyte. must be >=1
    #[argh(option)]
    fee_rate: Option<u64>,
}

pub async fn deposit(
    DepositArgs {
        alpen_address,
        fee_rate,
    }: DepositArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let requested_alpen_address = alpen_address
        .map(|a| {
            AlpenAddress::from_str(&a).user_error(format!(
                "Invalid Alpen address '{a}'. Must be an EVM-compatible address"
            ))
        })
        .transpose()?;
    let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
        .internal_error("Failed to load signet wallet")?;
    let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
        .user_error("Invalid Alpen endpoint URL. Check the config file")?;

    l1w.sync()
        .await
        .internal_error("Failed to sync signet wallet")?;

    let alpen_address = requested_alpen_address.unwrap_or(l2w.default_signer_address());
    println!(
        "Bridging {} to Alpen address {}",
        settings.bridge_in_amount.to_string().green(),
        alpen_address.to_string().cyan(),
    );

    let (secret_key, recovery_public_key) = even_kp(SECP256K1.generate_keypair(&mut OsRng));
    let recovery_public_key = recovery_public_key.x_only_public_key().0;

    println!(
        "Recovery public key: {}",
        encode(&recovery_public_key.serialize()).yellow()
    );

    let recovery_private_key = PrivateKey::new(secret_key.into(), settings.network);

    let bridge_in_desc = bridge_in_descriptor(
        settings.bridge_musig2_pubkey,
        recovery_private_key,
        settings.recover_delay,
    );

    let desc = bridge_in_desc
        .clone()
        .into_wallet_descriptor(l1w.secp_ctx(), settings.network)
        .expect("valid descriptor");

    let mut temp_wallet = Wallet::create_single(desc.clone())
        .network(settings.network)
        .create_wallet_no_persist()
        .expect("valid descriptor");

    let current_block_height = l1w
        .local_chain()
        .get_chain_tip()
        .expect("valid chain tip")
        .height;

    // Number of blocks after which the wallet actually enables recovery. This is mostly to account
    // for any reorgs that may happen at the recovery height.
    let recover_at_delay = settings.recover_delay + settings.finality_depth;

    let recover_at = current_block_height + recover_at_delay;

    let bridge_in_address = temp_wallet
        .reveal_next_address(KeychainKind::External)
        .address;

    println!(
        "Using {} as bridge in address",
        bridge_in_address.to_string().yellow()
    );

    let fee_rate = get_fee_rate(fee_rate, settings.signet_backend.as_ref()).await;
    log_fee_rate(&fee_rate);

    // Construct the DRT metadata OP_RETURN:
    // <magic_bytes>
    // <recovery_address_pk>
    // <alpen_address>
    let magic_bytes = settings.magic_bytes.as_bytes();
    let recovery_address_pk_bytes = recovery_public_key.serialize();
    let alpen_address_bytes = alpen_address.as_slice();
    let mut op_return_data = Vec::with_capacity(
        magic_bytes.len() + recovery_address_pk_bytes.len() + alpen_address_bytes.len(),
    );

    op_return_data.extend_from_slice(magic_bytes);
    op_return_data.extend_from_slice(&recovery_address_pk_bytes);
    op_return_data.extend_from_slice(alpen_address_bytes);

    // Convert to PushBytes (ensures length ≤ 80 bytes)
    let push_bytes = PushBytesBuf::try_from(op_return_data)
        .expect("conversion should succeed after length check");

    let mut psbt = {
        let mut builder = l1w.build_tx();
        // Important: the deposit won't be found by the sequencer if the order isn't correct.
        builder.ordering(TxOrdering::Untouched);
        builder.add_recipient(bridge_in_address.script_pubkey(), settings.bridge_in_amount);
        builder.add_data(&push_bytes);
        builder.fee_rate(fee_rate);
        match builder.finish() {
            Ok(psbt) => psbt,
            Err(CreateTxError::CoinSelection(e @ InsufficientFunds { .. })) => {
                return Err(DisplayedError::UserError(
                    "Failed to create bridge transaction".to_string(),
                    Box::new(e),
                ));
            }
            Err(e) => panic!("Unexpected error in creating PSBT: {e:?}"),
        }
    };
    l1w.sign(&mut psbt, Default::default())
        .expect("tx should be signed");
    println!("Built transaction");

    let tx = psbt.extract_tx().expect("tx should be signed and ready");

    let pb = ProgressBar::new_spinner().with_message("Saving output descriptor");
    pb.enable_steady_tick(Duration::from_millis(100));

    let mut desc_file = DescriptorRecovery::open(&seed, &settings.descriptor_db)
        .await
        .internal_error("Failed to open descriptor recovery file")?;
    desc_file
        .add_desc(recover_at, &bridge_in_desc)
        .await
        .internal_error("Failed to save recovery descriptor to recovery file")?;
    pb.finish_with_message("Saved output descriptor");

    let pb = ProgressBar::new_spinner().with_message("Broadcasting transaction");
    pb.enable_steady_tick(Duration::from_millis(100));
    settings
        .signet_backend
        .broadcast_tx(&tx)
        .await
        .internal_error("Failed to broadcast signet transaction")?;
    let txid = tx.compute_txid();
    pb.finish_with_message(
        OnchainObject::from(&txid)
            .with_maybe_explorer(settings.mempool_space_endpoint.as_deref())
            .pretty(),
    );
    println!("Expect transaction confirmation in ~{SIGNET_BLOCK_TIME:?}. Funds will take longer than this to be available on Alpen.");
    Ok(())
}

/// Generates a bridge-in descriptor for a given bridge public key and recovery address.
///
/// Returns a P2TR descriptor template for the bridge-in transaction.
///
/// # Implementation Details
///
/// This is a P2TR address that the key path spend is locked to the bridge aggregated public key
/// and the single script path spend is locked to the user's recovery address with a timelock of
fn bridge_in_descriptor(
    bridge_pubkey: XOnlyPublicKey,
    private_key: PrivateKey,
    recover_delay: u32,
) -> DescriptorTemplateOut {
    bdk_wallet::descriptor!(
        tr(bridge_pubkey,
            and_v(v:pk(private_key),older(recover_delay))
        )
    )
    .expect("valid descriptor")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bdk_wallet::{
        bitcoin::{secp256k1::SECP256K1, Network},
        keys::{DescriptorPublicKey, SinglePub, SinglePubKey},
        miniscript::{descriptor::TapTree, Descriptor, Miniscript},
    };
    use strata_primitives::constants::RECOVER_DELAY;

    use super::*;

    #[test]
    fn bridge_in_desc() {
        let bridge_pubkey = XOnlyPublicKey::from_str(
            "89f96f834e39766f97e245d70b27236681f741ae51c117df19761af7cb2f657e",
        )
        .expect("valid pubkey");

        let (secret_key, public_key) = SECP256K1.generate_keypair(&mut OsRng);

        let recovery_private_key = PrivateKey::new(secret_key, Network::Bitcoin);

        let (desc, _key_map, _network) =
            bridge_in_descriptor(bridge_pubkey, recovery_private_key, RECOVER_DELAY);
        assert!(desc.sanity_check().is_ok());
        let Descriptor::Tr(tr_desc) = desc else {
            panic!("should be taproot descriptor")
        };

        let expected_recovery_script = format!("and_v(v:pk({public_key}),older({RECOVER_DELAY}))",);

        let expected_taptree = TapTree::Leaf(Arc::new(
            Miniscript::from_str(&expected_recovery_script).expect("valid miniscript"),
        ));

        let expected_internal_key = DescriptorPublicKey::Single(SinglePub {
            origin: None,
            key: SinglePubKey::XOnly(bridge_pubkey),
        });

        assert_eq!(
            tr_desc.internal_key(),
            &expected_internal_key,
            "internal key should be the bridge pubkey"
        );

        assert_eq!(
            tr_desc.tap_tree().as_ref().expect("taptree to be present"),
            &expected_taptree,
            "tap tree should be the expected taptree"
        )
    }
}

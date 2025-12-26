use std::{str::FromStr, time::Duration};

use alloy::{primitives::Address as AlpenAddress, providers::WalletProvider};
use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{
        secp256k1::SECP256K1, Address as BitcoinAddress, Amount, FeeRate, Network, PrivateKey,
        Transaction, TxOut, XOnlyPublicKey,
    },
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
use strata_asm_txs_bridge_v1::deposit_request::{DrtHeaderAux, SubjectIdBytes};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::AccountSerial;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
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

/// Build and sign the deposit-request transaction with the SPS-50 OP_RETURN in output 0 and the
/// bridge output in 1.
fn build_deposit_request_tx(
    l1w: &mut Wallet,
    header_aux: &DrtHeaderAux,
    deposit_output: &TxOut,
    magic_bytes: MagicBytes,
    fee_rate: FeeRate,
) -> Result<Transaction, DisplayedError> {
    let drt_sps50_tag = header_aux.build_tag_data();

    let sps50_script = ParseConfig::new(magic_bytes)
        .encode_script_buf(&drt_sps50_tag.as_ref())
        .expect("drt metadata should be created");
    let mut builder = l1w.build_tx();
    // Important: the deposit won't be found by the sequencer if the order isn't correct.
    // Per SPS-50 spec: OP_RETURN must be at index 0, P2TR at index 1
    builder.ordering(TxOrdering::Untouched);
    builder.add_recipient(sps50_script, Amount::ZERO);
    builder.add_recipient(deposit_output.script_pubkey.clone(), deposit_output.value);
    builder.fee_rate(fee_rate);
    let mut psbt = match builder.finish() {
        Ok(psbt) => Ok(psbt),
        Err(CreateTxError::CoinSelection(e @ InsufficientFunds { .. })) => {
            Err(DisplayedError::UserError(
                "Failed to create bridge transaction".to_string(),
                Box::new(e),
            ))
        }
        Err(e) => panic!("Unexpected error in creating PSBT: {e:?}"),
    }?;

    l1w.sign(&mut psbt, Default::default())
        .expect("tx should be signed");
    Ok(psbt.extract_tx().expect("tx should be signed and ready"))
}

/// Prepare the bridge-in descriptor, address, and SPS-50 aux data for a deposit request.
fn prepare_deposit_request(
    bridge_pubkey: XOnlyPublicKey,
    network: Network,
    recover_delay: u32,
    alpen_address: AlpenAddress,
    bridge_in_amount: Amount,
) -> (DescriptorTemplateOut, BitcoinAddress, DrtHeaderAux, TxOut) {
    let (secret_key, recovery_public_key) = even_kp(SECP256K1.generate_keypair(&mut OsRng));
    let recovery_public_key = recovery_public_key.x_only_public_key().0;
    let recovery_private_key = PrivateKey::new(secret_key.into(), network);

    let bridge_in_desc = bridge_in_descriptor(bridge_pubkey, recovery_private_key, recover_delay);
    let bridge_in_address = {
        let desc = bridge_in_desc
            .clone()
            .into_wallet_descriptor(SECP256K1, network)
            .expect("valid descriptor");
        let mut temp_wallet = Wallet::create_single(desc)
            .network(network)
            .create_wallet_no_persist()
            .expect("valid descriptor");
        temp_wallet
            .reveal_next_address(KeychainKind::External)
            .address
    };

    let alpen_subject_bytes =
        SubjectIdBytes::try_new(alpen_address.to_vec()).expect("must be valid subject bytes");
    let header_aux = DrtHeaderAux::new(
        recovery_public_key.serialize(),
        // Legacy: deposit intent supports a single execution environment (zero)
        AccountSerial::zero(),
        alpen_subject_bytes,
    );
    let deposit_output = TxOut {
        value: bridge_in_amount,
        script_pubkey: bridge_in_address.script_pubkey(),
    };
    (
        bridge_in_desc,
        bridge_in_address,
        header_aux,
        deposit_output,
    )
}

pub async fn deposit(
    DepositArgs {
        alpen_address,
        fee_rate,
    }: DepositArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let mut l1w = SignetWallet::new(
        &seed,
        settings.params.network,
        settings.signet_backend.clone(),
    )
    .internal_error("Failed to load signet wallet")?;
    let l2w = AlpenWallet::new(&seed, &settings.alpen_endpoint)
        .user_error("Invalid Alpen endpoint URL. Check the config file")?;

    l1w.sync()
        .await
        .internal_error("Failed to sync signet wallet")?;

    let requested_alpen_address = alpen_address
        .map(|a| {
            AlpenAddress::from_str(&a).user_error(format!(
                "Invalid Alpen address '{a}'. Must be an EVM-compatible address"
            ))
        })
        .transpose()?;
    let alpen_address = requested_alpen_address.unwrap_or(l2w.default_signer_address());
    let drt_amount = settings.params.deposit_amount + settings.bridge_fee;
    println!(
        "Bridging {} to Alpen address {}",
        drt_amount.to_string().green(),
        alpen_address.to_string().cyan(),
    );

    let (bridge_in_desc, bridge_in_address, header_aux, deposit_output) = prepare_deposit_request(
        settings.bridge_musig2_pubkey,
        settings.params.network,
        settings.params.recovery_delay,
        alpen_address,
        drt_amount,
    );

    println!(
        "Recovery public key: {}",
        encode(header_aux.recovery_pk()).yellow()
    );

    let current_block_height = l1w
        .local_chain()
        .get_chain_tip()
        .expect("valid chain tip")
        .height;

    // Number of blocks after which the wallet actually enables recovery. This is mostly to account
    // for any reorgs that may happen at the recovery height.
    let recover_at =
        current_block_height + settings.params.recovery_delay + settings.finality_depth;

    println!(
        "Using {} as bridge in address",
        bridge_in_address.to_string().yellow()
    );

    let fee_rate = get_fee_rate(fee_rate, settings.signet_backend.as_ref()).await;
    log_fee_rate(&fee_rate);

    let tx = build_deposit_request_tx(
        &mut l1w,
        &header_aux,
        &deposit_output,
        settings.params.magic_bytes,
        fee_rate,
    )?;
    println!("Built transaction");

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
        bitcoin::{bip32::Xpriv, secp256k1::SECP256K1, Amount, FeeRate, Network},
        keys::{DescriptorPublicKey, SinglePub, SinglePubKey},
        miniscript::{descriptor::TapTree, Descriptor, Miniscript},
    };
    use strata_asm_txs_bridge_v1::deposit_request::parse_drt;
    use strata_primitives::constants::RECOVER_DELAY;
    use strata_test_utils_btcio::BtcioTestHarness;

    use super::*;

    /// Populate the wallet with on-chain data by replaying blocks from the corepc node.
    fn sync_wallet_from_node(wallet: &mut Wallet, harness: &BtcioTestHarness) {
        let node = harness.bitcoind();
        let tip_height = node.client.get_block_count().expect("block count").0;
        for height in 1..=tip_height {
            let block_hash = node
                .client
                .get_block_hash(height)
                .expect("block hash")
                .0
                .parse()
                .expect("block hash parse");
            let block = node.client.get_block(block_hash).expect("block");
            wallet
                .apply_block(&block, height as u32)
                .expect("apply block");
        }
    }

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

    #[test]
    fn deposit_request_tx_parses_in_asm() {
        let bridge_pubkey = XOnlyPublicKey::from_str(
            "89f96f834e39766f97e245d70b27236681f741ae51c117df19761af7cb2f657e",
        )
        .expect("valid pubkey");
        let alpen_address = AlpenAddress::from_str("0x5400000000000000000000000000000000000001")
            .expect("valid Alpen address");

        let harness =
            BtcioTestHarness::new_with_coinbase_maturity().expect("bitcoind harness should start");

        let xpriv = Xpriv::new_master(Network::Regtest, &[0u8; 32]).expect("valid xpriv");
        let base_desc = format!("tr({xpriv}/86h/0h/0h");
        let external_desc = format!("{base_desc}/0/*)");
        let internal_desc = format!("{base_desc}/1/*)");
        let mut wallet = Wallet::create(external_desc, internal_desc)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .expect("valid test wallet");

        let fund_address = wallet.reveal_next_address(KeychainKind::External).address;
        // Fund and confirm the wallet so PSBT construction has spendable inputs.
        let node = harness.bitcoind();
        node.client
            .send_to_address(&fund_address, Amount::from_sat(500_000))
            .expect("funding transaction should be created");
        harness
            .mine_blocks_blocking(1, None)
            .expect("block should be mined");
        sync_wallet_from_node(&mut wallet, &harness);

        let bridge_in_amount = Amount::from_sat(100_000);
        let (_bridge_in_desc, bridge_in_address, header_aux, deposit_output) =
            prepare_deposit_request(
                bridge_pubkey,
                Network::Regtest,
                RECOVER_DELAY,
                alpen_address,
                bridge_in_amount,
            );

        let tx = build_deposit_request_tx(
            &mut wallet,
            &header_aux,
            &deposit_output,
            "alpn".parse().expect("valid magic bytes"),
            FeeRate::from_sat_per_vb(1).expect("valid fee rate"),
        )
        .expect("tx should be built");
        let parsed = parse_drt(&tx).expect("tx should parse as DRT");
        assert_eq!(parsed.header_aux(), &header_aux);

        let parsed_output = parsed.deposit_request_output().inner();
        assert_eq!(parsed_output.value, bridge_in_amount);
        assert_eq!(
            parsed_output.script_pubkey,
            bridge_in_address.script_pubkey()
        );
    }
}

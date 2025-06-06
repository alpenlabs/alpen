use std::{str::FromStr, time::Duration};

use alloy::{primitives::Address as AlpenAddress, providers::WalletProvider};
use argh::FromArgs;
use bdk_wallet::{
    bitcoin::{
        script::PushBytesBuf, taproot::LeafVersion, Address, ScriptBuf, TapNodeHash, XOnlyPublicKey,
    },
    chain::ChainOracle,
    coin_selection::InsufficientFunds,
    descriptor::IntoWalletDescriptor,
    error::CreateTxError,
    miniscript::{miniscript::Tap, Miniscript},
    template::DescriptorTemplateOut,
    KeychainKind, TxOrdering, Wallet,
};
use colored::Colorize;
use indicatif::ProgressBar;
use strata_primitives::constants::RECOVER_DELAY;

use crate::{
    alpen::AlpenWallet,
    constants::{BRIDGE_IN_AMOUNT, RECOVER_AT_DELAY, SIGNET_BLOCK_TIME},
    errors::{DisplayableError, DisplayedError},
    link::{OnchainObject, PrettyPrint},
    recovery::DescriptorRecovery,
    seed::Seed,
    settings::Settings,
    signet::{get_fee_rate, log_fee_rate, SignetWallet},
    taproot::{ExtractP2trPubkey, NotTaprootAddress},
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
    let recovery_address = l1w.reveal_next_address(KeychainKind::External).address;
    let recovery_address_pk = recovery_address
        .extract_p2tr_pubkey()
        .expect("internal keychain should be taproot");
    l1w.persist()
        .internal_error("Failed to persist signet wallet")?;

    let alpen_address = requested_alpen_address.unwrap_or(l2w.default_signer_address());
    println!(
        "Bridging {} to Alpen address {}",
        BRIDGE_IN_AMOUNT.to_string().green(),
        alpen_address.to_string().cyan(),
    );

    println!(
        "Recovery address: {}",
        recovery_address.to_string().yellow()
    );

    let (bridge_in_desc, _recovery_script, _recovery_script_hash) =
        bridge_in_descriptor(settings.bridge_musig2_pubkey, recovery_address)
            .expect("valid bridge in descriptor");

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

    let recover_at = current_block_height + RECOVER_AT_DELAY;

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
    let recovery_address_pk_bytes = recovery_address_pk.serialize();
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
        builder.add_recipient(bridge_in_address.script_pubkey(), BRIDGE_IN_AMOUNT);
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
    recovery_address: Address,
) -> Result<(DescriptorTemplateOut, ScriptBuf, TapNodeHash), NotTaprootAddress> {
    let recovery_xonly_pubkey = recovery_address.extract_p2tr_pubkey()?;

    let desc = bdk_wallet::descriptor!(
        tr(bridge_pubkey,
            and_v(v:pk(recovery_xonly_pubkey),older(RECOVER_DELAY))
        )
    )
    .expect("valid descriptor");

    // we have to do this to obtain the script hash
    // i have tried to extract it directly from the desc above
    // it is a massive pita
    let recovery_script = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
        "and_v(v:pk({recovery_xonly_pubkey}),older({RECOVER_DELAY}))"
    ))
    .expect("valid recovery script")
    .encode();

    let recovery_script_hash = TapNodeHash::from_script(&recovery_script, LeafVersion::TapScript);

    Ok((desc, recovery_script, recovery_script_hash))
}

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::{consensus, secp256k1::SECP256K1, Network, Sequence};

    use super::*;

    #[test]
    fn bridge_in_descriptor_script() {
        pub(crate) const BRIDGE_MUSIG2_PUBKEY: &str =
            "14ced579c6a92533fa68ccc16da93b41073993cfc6cc982320645d8e9a63ee65";

        let bridge_musig2_pubkey = BRIDGE_MUSIG2_PUBKEY.parse::<XOnlyPublicKey>().unwrap();
        let internal_recovery_pubkey = XOnlyPublicKey::from_slice(&[2u8; 32]).unwrap();
        let recovery_address =
            Address::p2tr(SECP256K1, internal_recovery_pubkey, None, Network::Bitcoin);
        let external_recovery_pubkey = recovery_address.extract_p2tr_pubkey().unwrap();
        let sequence = Sequence::from_consensus(RECOVER_DELAY);
        let sequence_hex = consensus::encode::serialize_hex(&sequence);

        let (_bridge_in_descriptor, recovery_script, _recovery_script_hash) =
            bridge_in_descriptor(bridge_musig2_pubkey, recovery_address).unwrap();

        let expected = format!(
            "OP_PUSHBYTES_32 {external_recovery_pubkey} OP_CHECKSIGVERIFY OP_PUSHBYTES_2 {sequence_hex:.4} OP_CSV"
        );
        let got = recovery_script.to_asm_string();
        assert_eq!(got, expected);
    }
}

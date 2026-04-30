//! CLI command for creating and broadcasting an EE predicate admin update.

use std::{slice, str::FromStr, thread, time::Duration};

use anyhow::{bail, Context};
use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Client, RpcApi};
use bdk_wallet::{
    bitcoin::{
        absolute::LockTime,
        bip32::Xpriv,
        blockdata::script,
        consensus::serialize,
        key::UntweakedKeypair,
        secp256k1::{schnorr::Signature, Message, SecretKey, XOnlyPublicKey, SECP256K1},
        sighash::{Prevouts, SighashCache, TapSighashType},
        taproot::{ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootSpendInfo},
        transaction::Version,
        Address, Amount, FeeRate, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    },
    KeychainKind, TxOrdering,
};
use serde_json::json;
use ssz::Encode as _;
use strata_asm_params::Role;
use strata_asm_proto_admin_txs::{
    actions::{
        updates::predicate::{PredicateUpdate, ProofType},
        MultisigAction, UpdateAction,
    },
    parser::SignedPayload,
    test_utils::create_signature_set,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_l1_txfmt::ParseConfig;
use strata_predicate::PredicateKey;

use crate::{
    constants::{MAGIC_BYTES, NETWORK},
    taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Create and broadcast an EE predicate admin update.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-ee-predicate-update")]
pub struct CreateEePredicateUpdateArgs {
    /// update sequence number (use 1 for the first update, then increment)
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `AlwaysAccept`, `NeverAccept`, `Bip340Schnorr:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// admin xpriv used to sign the update action
    #[argh(option)]
    pub admin_xpriv: String,

    /// bitcoin RPC URL
    #[argh(option)]
    pub btc_url: String,

    /// bitcoin RPC username
    #[argh(option)]
    pub btc_user: String,

    /// bitcoin RPC password
    #[argh(option)]
    pub btc_password: String,

    /// fee rate in sat/vB for commit/reveal txs (default 2)
    #[argh(option, default = "2")]
    pub fee_rate: u64,

    /// commit output value in sats (default 20000)
    #[argh(option, default = "20_000")]
    pub commit_output_sats: u64,
}

fn parse_predicate_key(value: &str) -> Result<PredicateKey, String> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|e| e.to_string())
}

/// Minimum non-dust value for the reveal output.
const MIN_REVEAL_OUTPUT_SATS: u64 = 546;
const DEFAULT_RETRY_COUNT: usize = 5;
const RETRY_SLEEP_MS: u64 = 200;

pub(crate) fn create_ee_predicate_update(
    args: CreateEePredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let (commit_tx, reveal_tx) =
        build_admin_commit_reveal_pair(&args).internal_error("failed to build admin tx pair")?;

    let client = new_bitcoind_client(
        &args.btc_url,
        None,
        Some(&args.btc_user),
        Some(&args.btc_password),
    )
    .internal_error("failed to create bitcoind RPC client")?;

    let commit_txid =
        broadcast_tx(&client, &commit_tx).internal_error("failed to broadcast commit tx")?;
    let reveal_txid = broadcast_reveal_with_retry(&client, &reveal_tx)
        .internal_error("failed to broadcast reveal tx")?;

    println!(
        "{}",
        json!({
            "commit_txid": commit_txid,
            "reveal_txid": reveal_txid,
        })
    );

    Ok(())
}

// TODO(STR-3191): deduplicate ennvelope commit/reveal transaction
fn build_admin_commit_reveal_pair(
    args: &CreateEePredicateUpdateArgs,
) -> anyhow::Result<(Transaction, Transaction)> {
    if args.commit_output_sats <= MIN_REVEAL_OUTPUT_SATS {
        bail!("commit_output_sats must be > {MIN_REVEAL_OUTPUT_SATS}");
    }

    let xpriv = Xpriv::from_str(&args.admin_xpriv).context("invalid admin xpriv")?;
    let admin_secret_key = xpriv.private_key;

    let action = build_ee_update_action(args.predicate.clone());
    let signed_payload = create_signed_payload(action.clone(), args.seq_no, &admin_secret_key);

    let envelope_bytes = signed_payload.as_ssz_bytes();
    let (envelope_keypair, envelope_xonly) = generate_keypair(admin_secret_key)?;
    let (reveal_script, taproot_spend_info, reveal_address) =
        build_reveal_script_and_address(&envelope_bytes, envelope_xonly)?;

    let tag_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&action.tag().as_ref())
        .context("failed to build SPS-50 script")?;

    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        &args.btc_url,
        None,
        Some(&args.btc_user),
        Some(&args.btc_password),
    )?;
    sync_wallet(&mut wallet, &client)?;

    let fee_rate = FeeRate::from_sat_per_vb_unchecked(args.fee_rate);
    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder.ordering(TxOrdering::Untouched);
        builder.add_recipient(
            reveal_address.script_pubkey(),
            Amount::from_sat(args.commit_output_sats),
        );
        builder.fee_rate(fee_rate);
        builder.finish().context("failed to build commit tx")?
    };

    wallet
        .sign(&mut psbt, Default::default())
        .context("failed to sign commit tx")?;

    let commit_tx = psbt.extract_tx().context("failed to finalize commit tx")?;

    let reveal_output = commit_tx
        .output
        .iter()
        .position(|o| o.script_pubkey == reveal_address.script_pubkey())
        .context("commit tx is missing reveal output")?;

    let reveal_prevout = commit_tx.output[reveal_output].clone();
    let recipient = wallet.peek_address(KeychainKind::External, 0).address;

    let control_block = taproot_spend_info
        .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
        .context("failed to build control block")?;

    let mut reveal_tx =
        build_reveal_transaction_template(&commit_tx, reveal_output as u32, recipient, &tag_script);

    // Set output value so the fee tracks the requested fee rate for this witness shape.
    let vsize = estimate_reveal_vsize(reveal_tx.clone(), &reveal_script, &control_block);
    let fee = vsize as u64 * args.fee_rate;
    let input_sats = reveal_prevout.value.to_sat();
    if input_sats <= fee + MIN_REVEAL_OUTPUT_SATS {
        bail!(
            "commit output too small for reveal tx: input={input_sats}, required>{}",
            fee + MIN_REVEAL_OUTPUT_SATS
        );
    }
    reveal_tx.output[1].value = Amount::from_sat(input_sats - fee);

    sign_reveal_transaction(
        &mut reveal_tx,
        &reveal_prevout,
        &reveal_script,
        &taproot_spend_info,
        &envelope_keypair,
    )?;

    Ok((commit_tx, reveal_tx))
}

fn build_ee_update_action(key: PredicateKey) -> MultisigAction {
    let update = PredicateUpdate::new(key, ProofType::EeStf);
    MultisigAction::Update(UpdateAction::from(update))
}

fn create_signed_payload(
    action: MultisigAction,
    seq_no: u64,
    admin_secret_key: &SecretKey,
) -> SignedPayload {
    let signatures = create_signature_set(
        slice::from_ref(admin_secret_key),
        &[0],
        &action,
        Role::AlpenAdministrator,
        seq_no,
    );
    SignedPayload::new(seq_no, action, signatures)
}

fn generate_keypair(secret_key: SecretKey) -> anyhow::Result<(UntweakedKeypair, XOnlyPublicKey)> {
    let keypair = UntweakedKeypair::from_seckey_slice(SECP256K1, &secret_key.secret_bytes())
        .context("failed to create keypair")?;
    let xonly = XOnlyPublicKey::from_keypair(&keypair).0;
    Ok((keypair, xonly))
}

fn build_reveal_script_and_address(
    envelope_bytes: &[u8],
    xonly_pubkey: XOnlyPublicKey,
) -> anyhow::Result<(ScriptBuf, TaprootSpendInfo, Address)> {
    let envelope_chunks = vec![envelope_bytes.to_vec()];
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&xonly_pubkey.serialize())
        .context("failed to build envelope script")?
        .add_envelopes(&envelope_chunks)
        .context("failed to add envelope bytes")?
        .build_without_min_check()
        .context("failed to finalize envelope script")?;

    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .context("failed to add taproot leaf")?
        .finalize(SECP256K1, xonly_pubkey)
        .map_err(|e| anyhow::anyhow!("failed to finalize taproot tree: {e:?}"))?;

    let reveal_address = Address::p2tr(
        SECP256K1,
        xonly_pubkey,
        taproot_spend_info.merkle_root(),
        NETWORK,
    );

    Ok((reveal_script, taproot_spend_info, reveal_address))
}

fn build_reveal_transaction_template(
    commit_tx: &Transaction,
    reveal_vout: u32,
    recipient: Address,
    tag_script: &ScriptBuf,
) -> Transaction {
    Transaction {
        lock_time: LockTime::ZERO,
        version: Version(2),
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: commit_tx.compute_txid(),
                vout: reveal_vout,
            },
            script_sig: script::Builder::new().into_script(),
            witness: Witness::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        }],
        output: vec![
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: tag_script.clone(),
            },
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: recipient.script_pubkey(),
            },
        ],
    }
}

fn estimate_reveal_vsize(
    mut reveal_tx: Transaction,
    reveal_script: &ScriptBuf,
    control_block: &ControlBlock,
) -> usize {
    reveal_tx.input[0].witness.push([0u8; 64]);
    reveal_tx.input[0].witness.push(reveal_script);
    reveal_tx.input[0].witness.push(control_block.serialize());
    reveal_tx.vsize()
}

fn sign_reveal_transaction(
    reveal_tx: &mut Transaction,
    prevout: &TxOut,
    reveal_script: &ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
    keypair: &UntweakedKeypair,
) -> anyhow::Result<()> {
    let signature = compute_reveal_signature(reveal_tx, prevout, reveal_script, keypair)?;

    let control_block = taproot_spend_info
        .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
        .context("failed to create control block")?;

    let witness = &mut reveal_tx.input[0].witness;
    witness.push(signature.as_ref());
    witness.push(reveal_script);
    witness.push(control_block.serialize());
    Ok(())
}

fn compute_reveal_signature(
    reveal_tx: &Transaction,
    prevout: &TxOut,
    reveal_script: &ScriptBuf,
    keypair: &UntweakedKeypair,
) -> anyhow::Result<Signature> {
    let mut sighash_cache = SighashCache::new(reveal_tx);
    let sighash = sighash_cache
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(slice::from_ref(prevout)),
            TapLeafHash::from_script(reveal_script, LeafVersion::TapScript),
            TapSighashType::Default,
        )
        .context("failed to compute reveal sighash")?;

    let msg =
        Message::from_digest_slice(sighash.as_ref()).context("invalid reveal sighash message")?;

    Ok(SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair))
}

fn broadcast_tx(client: &Client, tx: &Transaction) -> anyhow::Result<String> {
    let raw_hex = hex::encode(serialize(tx));
    client
        .call("sendrawtransaction", &[serde_json::Value::String(raw_hex)])
        .context("failed to broadcast transaction")
}

fn broadcast_reveal_with_retry(client: &Client, reveal_tx: &Transaction) -> anyhow::Result<String> {
    for attempt in 0..DEFAULT_RETRY_COUNT {
        match broadcast_tx(client, reveal_tx) {
            Ok(txid) => return Ok(txid),
            Err(err) => {
                let msg = err.to_string().to_lowercase();
                let should_retry = msg.contains("missing") || msg.contains("invalid input");
                if should_retry && attempt + 1 < DEFAULT_RETRY_COUNT {
                    thread::sleep(Duration::from_millis(RETRY_SLEEP_MS));
                    continue;
                }
                return Err(err);
            }
        }
    }

    bail!("exhausted reveal broadcast retries")
}

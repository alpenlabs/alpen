//! CLI commands for creating and broadcasting predicate admin updates.

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
use serde_json::{json, Value};
use ssz::Encode as _;
use strata_asm_proto_admin_txs::{
    actions::{
        updates::{EeStfVkUpdate, OlStfVkUpdate},
        MultisigAction, UpdateAction,
    },
    parser::SignedPayload,
    signing_message::SigningMessage,
    test_utils::sign_ecdsa_bip137,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_crypto::threshold_signature::{IndexedSignature, SignatureSet};
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

    /// admin xpriv used to sign the update action; repeat once per signer for a
    /// threshold (M-of-N) update. Pairs positionally with `--signer-index`.
    #[argh(option)]
    pub admin_xpriv: Vec<String>,

    /// member index (position in the admin key set) for the matching
    /// `--admin-xpriv`; repeat once per signer. Defaults to `[0]` for a single
    /// signer when omitted (1-of-N backward compatibility).
    #[argh(option)]
    pub signer_index: Vec<u8>,

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

/// Broadcast an OL checkpoint predicate update.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-checkpoint-predicate-update")]
pub struct CreateCheckpointPredicateUpdateArgs {
    /// update sequence number (use 1 for the first update, then increment)
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `AlwaysAccept`, `NeverAccept`, `Sp1Groth16:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// admin xpriv used to sign the update action; repeat once per signer for a
    /// threshold (M-of-N) update. Pairs positionally with `--signer-index`.
    #[argh(option)]
    pub admin_xpriv: Vec<String>,

    /// member index (position in the admin key set) for the matching
    /// `--admin-xpriv`; repeat once per signer. Defaults to `[0]` for a single
    /// signer when omitted (1-of-N backward compatibility).
    #[argh(option)]
    pub signer_index: Vec<u8>,

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
    serde_json::from_value(Value::String(value.to_owned())).map_err(|e| e.to_string())
}

/// Minimum non-dust value for the reveal output.
const MIN_REVEAL_OUTPUT_SATS: u64 = 546;
const DEFAULT_RETRY_COUNT: usize = 5;
const RETRY_SLEEP_MS: u64 = 200;

#[derive(Clone, Copy, Debug)]
enum PredicateUpdateTarget {
    EeStfVk,
    OlStfVk,
}

/// A single admin signer: an xpriv and its member index in the admin key set.
#[derive(Clone, Debug)]
struct AdminSigner {
    xpriv: String,
    signer_index: u8,
}

#[derive(Debug)]
struct PredicateUpdateRequest {
    seq_no: u64,
    predicate: PredicateKey,
    signers: Vec<AdminSigner>,
    btc_url: String,
    btc_user: String,
    btc_password: String,
    fee_rate: u64,
    commit_output_sats: u64,
    target: PredicateUpdateTarget,
}

/// Pairs the repeatable `--admin-xpriv` and `--signer-index` args into signers.
///
/// When `--signer-index` is omitted, it defaults to `[0]` and exactly one
/// `--admin-xpriv` is required, preserving single-sig (1-of-N) behaviour.
/// Otherwise the two lists must be non-empty and of equal length.
fn build_signers(
    admin_xprivs: Vec<String>,
    mut signer_indices: Vec<u8>,
) -> Result<Vec<AdminSigner>, DisplayedError> {
    if admin_xprivs.is_empty() {
        return Err(DisplayedError::UserError(
            "at least one --admin-xpriv is required".to_owned(),
            Box::new(()),
        ));
    }

    if signer_indices.is_empty() {
        if admin_xprivs.len() != 1 {
            return Err(DisplayedError::UserError(
                "--signer-index must be provided once per --admin-xpriv when supplying \
                 multiple signers"
                    .to_owned(),
                Box::new(()),
            ));
        }
        // Single-sig backward compatibility: default to member index 0.
        signer_indices = vec![0];
    }

    if admin_xprivs.len() != signer_indices.len() {
        return Err(DisplayedError::UserError(
            format!(
                "--admin-xpriv (count {}) and --signer-index (count {}) must match",
                admin_xprivs.len(),
                signer_indices.len()
            ),
            Box::new(()),
        ));
    }

    Ok(admin_xprivs
        .into_iter()
        .zip(signer_indices)
        .map(|(xpriv, signer_index)| AdminSigner {
            xpriv,
            signer_index,
        })
        .collect())
}

pub(crate) fn create_ee_predicate_update(
    args: CreateEePredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let signers = build_signers(args.admin_xpriv, args.signer_index)?;
    create_predicate_update(PredicateUpdateRequest {
        seq_no: args.seq_no,
        predicate: args.predicate,
        signers,
        btc_url: args.btc_url,
        btc_user: args.btc_user,
        btc_password: args.btc_password,
        fee_rate: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
        target: PredicateUpdateTarget::EeStfVk,
    })
}

pub(crate) fn create_checkpoint_predicate_update(
    args: CreateCheckpointPredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let signers = build_signers(args.admin_xpriv, args.signer_index)?;
    create_predicate_update(PredicateUpdateRequest {
        seq_no: args.seq_no,
        predicate: args.predicate,
        signers,
        btc_url: args.btc_url,
        btc_user: args.btc_user,
        btc_password: args.btc_password,
        fee_rate: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
        target: PredicateUpdateTarget::OlStfVk,
    })
}

fn create_predicate_update(args: PredicateUpdateRequest) -> Result<(), DisplayedError> {
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

// TODO(STR-3191): deduplicate envelope commit/reveal transaction
fn build_admin_commit_reveal_pair(
    args: &PredicateUpdateRequest,
) -> anyhow::Result<(Transaction, Transaction)> {
    if args.commit_output_sats <= MIN_REVEAL_OUTPUT_SATS {
        bail!("commit_output_sats must be > {MIN_REVEAL_OUTPUT_SATS}");
    }

    // Derive each signer's secret key together with its admin member index.
    let indexed_secret_keys = derive_indexed_secret_keys(&args.signers)?;

    let action = build_predicate_update_action(args.predicate.clone(), args.target);
    let signed_payload = create_signed_payload(action.clone(), args.seq_no, &indexed_secret_keys)?;

    let envelope_bytes = signed_payload.as_ssz_bytes();
    // The envelope keypair only controls the taproot reveal spend; it is unrelated
    // to the admin multisig. Reuse the first signer's key for it.
    let (envelope_secret_key, _) = indexed_secret_keys[0];
    let (envelope_keypair, envelope_xonly) = generate_keypair(envelope_secret_key)?;
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

    let fee_rate_sat_per_vb = u32::try_from(args.fee_rate).context("fee rate exceeds u32 range")?;
    let fee_rate = FeeRate::from_sat_per_vb_u32(fee_rate_sat_per_vb);
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

fn build_predicate_update_action(
    key: PredicateKey,
    target: PredicateUpdateTarget,
) -> MultisigAction {
    let update = match target {
        PredicateUpdateTarget::EeStfVk => UpdateAction::EeStfVk(EeStfVkUpdate::new(key)),
        PredicateUpdateTarget::OlStfVk => UpdateAction::OlStfVk(OlStfVkUpdate::new(key)),
    };
    MultisigAction::Update(update)
}

/// Derives the secret key for each configured signer, paired with its admin
/// member index.
fn derive_indexed_secret_keys(signers: &[AdminSigner]) -> anyhow::Result<Vec<(SecretKey, u8)>> {
    signers
        .iter()
        .map(|signer| {
            let xpriv = Xpriv::from_str(&signer.xpriv).context("invalid admin xpriv")?;
            Ok((xpriv.private_key, signer.signer_index))
        })
        .collect()
}

/// Builds the threshold [`SignatureSet`] for `action` and wraps it in a
/// [`SignedPayload`].
///
/// Each provided `(secret_key, member_index)` produces one [`IndexedSignature`]
/// over the canonical admin signing message, mirroring exactly what the ASM
/// admin subprotocol verifies. Building the signatures directly (rather than via
/// `create_signature_set`, which indexes `privkeys[index]`) lets us pair each
/// key with its own member index without constructing a sparse key slice.
fn create_signed_payload(
    action: MultisigAction,
    seq_no: u64,
    indexed_secret_keys: &[(SecretKey, u8)],
) -> anyhow::Result<SignedPayload> {
    let message_hash = SigningMessage::for_action(&action, seq_no).compute_sighash();
    let signatures: Vec<IndexedSignature> = indexed_secret_keys
        .iter()
        .map(|(secret_key, index)| {
            let sig = sign_ecdsa_bip137(&message_hash.0, secret_key);
            IndexedSignature::new(*index, sig)
        })
        .collect();
    let signature_set = SignatureSet::new(signatures)
        .map_err(|e| anyhow::anyhow!("failed to build signature set: {e}"))?;
    Ok(SignedPayload::new(seq_no, action, signature_set))
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

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use bdk_wallet::bitcoin::{
        bip32::Xpriv,
        secp256k1::{PublicKey, SECP256K1},
        NetworkKind,
    };
    use strata_asm_proto_admin_txs::signing_message::SigningMessage;
    use strata_crypto::{
        keys::compressed::CompressedPublicKey,
        threshold_signature::{
            verify_threshold_signatures, ThresholdConfig, ThresholdSignatureError,
        },
    };
    use strata_predicate::PredicateKey;

    use super::*;

    /// Builds a deterministic master xpriv from a single-byte seed.
    fn test_xpriv(seed: u8) -> Xpriv {
        let seed_bytes = [seed; 32];
        Xpriv::new_master(NetworkKind::Test, &seed_bytes).expect("valid xpriv")
    }

    /// Builds a threshold-2 config over the member pubkeys derived from `xprivs`,
    /// ordered by member index.
    fn threshold2_config(xprivs: &[Xpriv]) -> ThresholdConfig {
        let pubkeys: Vec<CompressedPublicKey> = xprivs
            .iter()
            .map(|xpriv| {
                let pk = PublicKey::from_secret_key(SECP256K1, &xpriv.private_key);
                CompressedPublicKey::from(pk)
            })
            .collect();
        ThresholdConfig::try_new(pubkeys, NonZero::new(2).expect("non-zero threshold"))
            .expect("valid threshold config")
    }

    /// Assembles a [`SignedPayload`] for an `EeStfVk` predicate update from the
    /// given signers, exercising the full CLI signing path
    /// (arg pairing -> xpriv parse -> threshold signature set).
    fn signed_payload_for(xprivs: &[&Xpriv], signer_indices: &[u8], seq_no: u64) -> SignedPayload {
        let admin_xprivs: Vec<String> = xprivs.iter().map(|x| x.to_string()).collect();
        let signers =
            build_signers(admin_xprivs, signer_indices.to_vec()).expect("valid signer arg pairing");
        let indexed_secret_keys =
            derive_indexed_secret_keys(&signers).expect("derive signer secret keys");

        let action = build_predicate_update_action(
            PredicateKey::always_accept(),
            PredicateUpdateTarget::EeStfVk,
        );
        create_signed_payload(action, seq_no, &indexed_secret_keys)
            .expect("assemble signed payload")
    }

    /// 2-of-3: a quorum-sized signature set verifies against the threshold-2 config
    /// using the exact ASM verification API.
    #[test]
    fn two_of_three_signed_payload_is_accepted() {
        let seq_no = 1;
        let xprivs = [test_xpriv(1), test_xpriv(2), test_xpriv(3)];
        let config = threshold2_config(&xprivs);

        // Signers at member indices 0 and 2.
        let payload = signed_payload_for(&[&xprivs[0], &xprivs[2]], &[0, 2], seq_no);
        assert_eq!(payload.signatures.len(), 2);

        let message_hash = SigningMessage::for_action(&payload.action, seq_no).compute_sighash();
        let result =
            verify_threshold_signatures(&config, payload.signatures.signatures(), &message_hash.0);
        assert!(result.is_ok(), "2-of-3 must be accepted: {result:?}");
    }

    /// 1-of-3: a single signature is below the threshold and must be rejected by
    /// the ASM verification API.
    #[test]
    fn one_of_three_signed_payload_is_rejected() {
        let seq_no = 1;
        let xprivs = [test_xpriv(1), test_xpriv(2), test_xpriv(3)];
        let config = threshold2_config(&xprivs);

        // Single signer at member index 0 (backward-compatible default path).
        let payload = signed_payload_for(&[&xprivs[0]], &[], seq_no);
        assert_eq!(payload.signatures.len(), 1);

        let message_hash = SigningMessage::for_action(&payload.action, seq_no).compute_sighash();
        let result =
            verify_threshold_signatures(&config, payload.signatures.signatures(), &message_hash.0);
        assert!(
            matches!(
                result,
                Err(ThresholdSignatureError::InsufficientSignatures {
                    provided: 1,
                    required: 2
                })
            ),
            "1-of-3 must be rejected as insufficient: {result:?}"
        );
    }

    /// Mismatched `--admin-xpriv` / `--signer-index` counts are rejected at arg
    /// parsing time.
    #[test]
    fn mismatched_signer_arg_counts_are_rejected() {
        let xprivs = [test_xpriv(1), test_xpriv(2)];
        let admin_xprivs: Vec<String> = xprivs.iter().map(|x| x.to_string()).collect();
        // Two keys but only one index.
        let result = build_signers(admin_xprivs, vec![0]);
        assert!(result.is_err(), "mismatched counts must be rejected");
    }
}

//! CLI commands for creating and broadcasting predicate admin updates.
//!
//! Supports M-of-N threshold signing (local keys and/or externally produced BIP-137
//! signatures), any network/magic, a local dry-run of the on-chain threshold check against
//! the deployed signer config, and funding from a `bitcoind` wallet.

use std::{collections::HashMap, path::PathBuf, slice, str::FromStr, thread, time::Duration};

use anyhow::{bail, ensure, Context};
use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::{json::FundRawTransactionOptions, Auth, Client, RpcApi};
use bdk_wallet::{
    bitcoin::{
        absolute::LockTime,
        blockdata::script,
        consensus::serialize,
        key::UntweakedKeypair,
        secp256k1::{schnorr::Signature, Message, SecretKey, XOnlyPublicKey, SECP256K1},
        sighash::{Prevouts, SighashCache, TapSighashType},
        taproot::{ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootSpendInfo},
        transaction::Version,
        Address, Amount, FeeRate, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    },
    KeychainKind, TxOrdering,
};
use rand::RngCore;
use serde_json::{json, Value};
use strata_asm_admin_threshold_sig::{SignatureSet, ThresholdConfig};
use strata_asm_proto_admin_txs::{
    actions::{
        updates::{EeStfVkUpdate, OlStfVkUpdate},
        MultisigAction, UpdateAction,
    },
    parser::SignedPayload,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_envelope_fmt::EnvelopeScriptBuilder;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_predicate::PredicateKey;

use crate::{
    admin::{
        assemble_signatures, decode_reveal_tx, parse_admin_key, parse_external_signature,
        parse_secret_key, signing_message, threshold_config_from_addresses,
        threshold_config_from_asm_params, verify_signatures,
    },
    constants::{MAGIC_BYTES, NETWORK},
    taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Create and broadcast an EE predicate admin update (role: AlpenAdministrator).
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-ee-predicate-update")]
pub struct CreateEePredicateUpdateArgs {
    /// update sequence number; must be > the role's last accepted seqno (max gap per params)
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `AlwaysAccept`, `NeverAccept`, `Bip340Schnorr:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// admin xpriv used to sign as signer index 0 (alias for `--admin-key 0:<xpriv>`)
    #[argh(option)]
    pub admin_xpriv: Option<String>,

    /// signing key as `<signer_index>:<xpriv|32-byte hex>` (repeatable)
    #[argh(option)]
    pub admin_key: Vec<String>,

    /// external BIP-137 signature as `<signer_index>:<base64>` (repeatable)
    #[argh(option)]
    pub signature: Vec<String>,

    /// asm params JSON (the deployed `asmParams`) whose Admin role config verifies signatures
    #[argh(option)]
    pub signers_config: Option<PathBuf>,

    /// signer threshold (with `--signer-address`; alternative to `--signers-config`)
    #[argh(option)]
    pub threshold: Option<u8>,

    /// P2WPKH signer address in signer-index order (repeatable, with `--threshold`)
    #[argh(option)]
    pub signer_address: Vec<String>,

    /// bitcoin network: regtest (default), signet, testnet or bitcoin
    #[argh(option, default = "NETWORK", from_str_fn(parse_network))]
    pub network: Network,

    /// SPS-50 magic bytes, 4 ASCII chars (default ALPN)
    #[argh(option, default = "MAGIC_BYTES", from_str_fn(parse_magic))]
    pub magic: MagicBytes,

    /// fund the commit tx from this bitcoind wallet (required unless network is regtest)
    #[argh(option)]
    pub btc_wallet: Option<String>,

    /// build, verify and print the txs without broadcasting
    #[argh(switch)]
    pub dry_run: bool,

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

/// Broadcast an OL checkpoint predicate update (role: StrataAdministrator).
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-checkpoint-predicate-update")]
pub struct CreateCheckpointPredicateUpdateArgs {
    /// update sequence number; must be > the role's last accepted seqno (max gap per params)
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `AlwaysAccept`, `NeverAccept`, `Sp1Groth16:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// admin xpriv used to sign as signer index 0 (alias for `--admin-key 0:<xpriv>`)
    #[argh(option)]
    pub admin_xpriv: Option<String>,

    /// signing key as `<signer_index>:<xpriv|32-byte hex>` (repeatable)
    #[argh(option)]
    pub admin_key: Vec<String>,

    /// external BIP-137 signature as `<signer_index>:<base64>` (repeatable)
    #[argh(option)]
    pub signature: Vec<String>,

    /// asm params JSON (the deployed `asmParams`) whose Admin role config verifies signatures
    #[argh(option)]
    pub signers_config: Option<PathBuf>,

    /// signer threshold (with `--signer-address`; alternative to `--signers-config`)
    #[argh(option)]
    pub threshold: Option<u8>,

    /// P2WPKH signer address in signer-index order (repeatable, with `--threshold`)
    #[argh(option)]
    pub signer_address: Vec<String>,

    /// bitcoin network: regtest (default), signet, testnet or bitcoin
    #[argh(option, default = "NETWORK", from_str_fn(parse_network))]
    pub network: Network,

    /// SPS-50 magic bytes, 4 ASCII chars (default ALPN)
    #[argh(option, default = "MAGIC_BYTES", from_str_fn(parse_magic))]
    pub magic: MagicBytes,

    /// fund the commit tx from this bitcoind wallet (required unless network is regtest)
    #[argh(option)]
    pub btc_wallet: Option<String>,

    /// build, verify and print the txs without broadcasting
    #[argh(switch)]
    pub dry_run: bool,

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

/// Print the exact text admin signers must sign with Bitcoin `signmessage` (BIP-137).
///
/// The message goes to stdout byte-for-byte (no trailing newline); role and digest go to
/// stderr. Sign the TEXT, not the digest: wallets apply the signMessage prefix and hash.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "print-signing-message")]
pub struct PrintSigningMessageArgs {
    /// update kind: `ee-stf-vk` (AlpenAdministrator) or `ol-stf-vk` (StrataAdministrator)
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,

    /// update sequence number
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `Sp1Groth16:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// bitcoin network the chain is anchored to (default regtest)
    #[argh(option, default = "NETWORK", from_str_fn(parse_network))]
    pub network: Network,
}

fn parse_predicate_key(value: &str) -> Result<PredicateKey, String> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|e| e.to_string())
}

fn parse_network(value: &str) -> Result<Network, String> {
    Network::from_str(value).map_err(|e| e.to_string())
}

fn parse_magic(value: &str) -> Result<MagicBytes, String> {
    if !value.is_ascii() {
        return Err(format!("magic `{value}` must be ASCII"));
    }
    MagicBytes::from_str(value).map_err(|e| e.to_string())
}

fn parse_target(value: &str) -> Result<PredicateUpdateTarget, String> {
    match value {
        "ee-stf-vk" => Ok(PredicateUpdateTarget::EeStfVk),
        "ol-stf-vk" => Ok(PredicateUpdateTarget::OlStfVk),
        other => Err(format!(
            "unknown update `{other}` (expected ee-stf-vk|ol-stf-vk)"
        )),
    }
}

/// Minimum non-dust value for the reveal output.
const MIN_REVEAL_OUTPUT_SATS: u64 = 546;
const DEFAULT_RETRY_COUNT: usize = 5;
const RETRY_SLEEP_MS: u64 = 200;

/// Which predicate an admin update targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredicateUpdateTarget {
    /// EE STF verifying key (AlpenAdministrator).
    EeStfVk,
    /// OL STF / checkpoint verifying key (StrataAdministrator).
    OlStfVk,
}

#[derive(Debug)]
struct PredicateUpdateRequest {
    seq_no: u64,
    predicate: PredicateKey,
    admin_xpriv: Option<String>,
    admin_key: Vec<String>,
    signature: Vec<String>,
    signers_config: Option<PathBuf>,
    threshold: Option<u8>,
    signer_address: Vec<String>,
    network: Network,
    magic: MagicBytes,
    btc_wallet: Option<String>,
    dry_run: bool,
    btc_url: String,
    btc_user: String,
    btc_password: String,
    fee_rate: u64,
    commit_output_sats: u64,
    target: PredicateUpdateTarget,
}

pub(crate) fn create_ee_predicate_update(
    args: CreateEePredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    create_predicate_update(PredicateUpdateRequest {
        seq_no: args.seq_no,
        predicate: args.predicate,
        admin_xpriv: args.admin_xpriv,
        admin_key: args.admin_key,
        signature: args.signature,
        signers_config: args.signers_config,
        threshold: args.threshold,
        signer_address: args.signer_address,
        network: args.network,
        magic: args.magic,
        btc_wallet: args.btc_wallet,
        dry_run: args.dry_run,
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
    create_predicate_update(PredicateUpdateRequest {
        seq_no: args.seq_no,
        predicate: args.predicate,
        admin_xpriv: args.admin_xpriv,
        admin_key: args.admin_key,
        signature: args.signature,
        signers_config: args.signers_config,
        threshold: args.threshold,
        signer_address: args.signer_address,
        network: args.network,
        magic: args.magic,
        btc_wallet: args.btc_wallet,
        dry_run: args.dry_run,
        btc_url: args.btc_url,
        btc_user: args.btc_user,
        btc_password: args.btc_password,
        fee_rate: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
        target: PredicateUpdateTarget::OlStfVk,
    })
}

pub(crate) fn print_signing_message(args: PrintSigningMessageArgs) -> Result<(), DisplayedError> {
    let action = build_predicate_update_action(args.predicate, args.update);
    let (message, digest) = signing_message(&action, args.seq_no, args.network);
    print!("{}", message.as_str());
    eprintln!();
    eprintln!("role: {}", action.required_role());
    eprintln!("network: {}", args.network);
    eprintln!(
        "signmessage digest (cross-check only, do not sign): {}",
        hex::encode(digest)
    );
    Ok(())
}

fn create_predicate_update(args: PredicateUpdateRequest) -> Result<(), DisplayedError> {
    let signed = sign_and_verify(&args).user_error("admin signature check failed")?;
    let (commit_tx, reveal_tx) = build_admin_commit_reveal_pair(&args, &signed)
        .internal_error("failed to build admin tx pair")?;

    let decoded = decode_reveal_tx(&reveal_tx, args.magic, &signed.payload)
        .internal_error("built reveal tx does not decode to the signed payload")?;

    if args.dry_run {
        println!(
            "{}",
            json!({
                "dry_run": true,
                "network": args.network.to_string(),
                "magic": args.magic.to_string(),
                "role": decoded.action.required_role().to_string(),
                "seqno": decoded.seqno,
                "signing_message": signed.message,
                "signing_digest": hex::encode(signed.digest),
                "threshold_verified": signed.verified,
                "action": format!("{:?}", decoded.action),
                "signatures": decoded.signatures.signatures().iter().map(|s| json!({
                    "index": s.index(),
                    "header": s.recovery_id(),
                    "compact": hex::encode(s.compact()),
                })).collect::<Vec<_>>(),
                "commit_txid": commit_tx.compute_txid().to_string(),
                "commit_tx": hex::encode(serialize(&commit_tx)),
                "reveal_txid": reveal_tx.compute_txid().to_string(),
                "reveal_tx": hex::encode(serialize(&reveal_tx)),
            })
        );
        return Ok(());
    }

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

/// A signed admin payload plus what was signed.
#[derive(Debug)]
struct SignedAdminPayload {
    payload: SignedPayload,
    message: String,
    digest: [u8; 32],
    /// Whether the signatures were checked against a signer config.
    verified: bool,
}

/// Builds the action, collects signatures and, when a signer config is given, runs the
/// on-chain threshold check. Touches no RPC, so it fails fast before any funding.
fn sign_and_verify(args: &PredicateUpdateRequest) -> anyhow::Result<SignedAdminPayload> {
    let action = build_predicate_update_action(args.predicate.clone(), args.target);
    let (message, digest) = signing_message(&action, args.seq_no, args.network);

    let mut keys = Vec::new();
    if let Some(xpriv) = &args.admin_xpriv {
        keys.push((0, parse_secret_key(xpriv).context("invalid --admin-xpriv")?));
    }
    for spec in &args.admin_key {
        keys.push(parse_admin_key(spec)?);
    }
    let external = args
        .signature
        .iter()
        .map(|s| parse_external_signature(s))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let sigs = assemble_signatures(&keys, external, &digest)?;

    let config = load_signer_config(args, &action)?;
    let verified = match &config {
        Some(config) => {
            verify_signatures(config, &sigs, &digest, args.network)?;
            true
        }
        None => {
            ensure!(
                args.network == Network::Regtest,
                "--signers-config or --threshold/--signer-address is required on {}",
                args.network
            );
            eprintln!("warning: no signer config given; signatures NOT verified");
            false
        }
    };

    let signatures = SignatureSet::new(sigs).context("invalid signature set")?;
    Ok(SignedAdminPayload {
        payload: SignedPayload::new(args.seq_no, action, signatures),
        message: message.as_str().to_owned(),
        digest,
        verified,
    })
}

fn load_signer_config(
    args: &PredicateUpdateRequest,
    action: &MultisigAction,
) -> anyhow::Result<Option<ThresholdConfig>> {
    match (
        &args.signers_config,
        args.threshold,
        args.signer_address.is_empty(),
    ) {
        (Some(_), Some(_), _) | (Some(_), _, false) => {
            bail!("use either --signers-config or --threshold/--signer-address, not both")
        }
        (Some(path), None, true) => {
            threshold_config_from_asm_params(path, action.required_role(), args.network, args.magic)
                .map(Some)
        }
        (None, Some(threshold), false) => {
            threshold_config_from_addresses(&args.signer_address, threshold, args.network).map(Some)
        }
        (None, Some(_), true) | (None, None, false) => {
            bail!("--threshold and --signer-address must be given together")
        }
        (None, None, true) => Ok(None),
    }
}

// TODO(STR-3191): deduplicate envelope commit/reveal transaction
fn build_admin_commit_reveal_pair(
    args: &PredicateUpdateRequest,
    signed: &SignedAdminPayload,
) -> anyhow::Result<(Transaction, Transaction)> {
    if args.commit_output_sats <= MIN_REVEAL_OUTPUT_SATS {
        bail!("commit_output_sats must be > {MIN_REVEAL_OUTPUT_SATS}");
    }

    let reveal = RevealParts::new(&signed.payload, args.magic, args.network)?;

    let fee_rate_sat_per_vb = u32::try_from(args.fee_rate).context("fee rate exceeds u32 range")?;
    let (commit_tx, recipient) = match &args.btc_wallet {
        Some(wallet) => fund_commit_with_bitcoind_wallet(args, wallet, &reveal.address)?,
        None => {
            ensure!(
                args.network == Network::Regtest,
                "--btc-wallet is required on {} (built-in test wallet is regtest-only)",
                args.network
            );
            fund_commit_with_test_wallet(args, &reveal.address, fee_rate_sat_per_vb)?
        }
    };

    let reveal_tx = reveal.build_signed_reveal_tx(&commit_tx, recipient, args.fee_rate)?;
    Ok((commit_tx, reveal_tx))
}

/// Everything needed to spend the commit output into the admin reveal tx.
#[derive(Debug)]
struct RevealParts {
    tag_script: ScriptBuf,
    reveal_script: ScriptBuf,
    taproot_spend_info: TaprootSpendInfo,
    address: Address,
    keypair: UntweakedKeypair,
}

impl RevealParts {
    fn new(payload: &SignedPayload, magic: MagicBytes, network: Network) -> anyhow::Result<Self> {
        let tag_script = ParseConfig::new(magic)
            .encode_script_buf(&payload.action.tag().as_ref())
            .context("failed to build SPS-50 script")?;
        let envelope_bytes = payload.clone().into_envelope_bytes();

        // The envelope key only authorizes spending the commit output; ASM ignores it, so an
        // ephemeral key decouples tx construction from the admin signers.
        let (keypair, xonly) = generate_keypair(random_secret_key()?)?;
        let (reveal_script, taproot_spend_info, address) =
            build_reveal_script_and_address(&envelope_bytes, xonly, network)?;
        Ok(Self {
            tag_script,
            reveal_script,
            taproot_spend_info,
            address,
            keypair,
        })
    }

    fn build_signed_reveal_tx(
        &self,
        commit_tx: &Transaction,
        recipient: Address,
        fee_rate: u64,
    ) -> anyhow::Result<Transaction> {
        let reveal_output = commit_tx
            .output
            .iter()
            .position(|o| o.script_pubkey == self.address.script_pubkey())
            .context("commit tx is missing reveal output")?;

        let reveal_prevout = commit_tx.output[reveal_output].clone();

        let control_block = self
            .taproot_spend_info
            .control_block(&(self.reveal_script.clone(), LeafVersion::TapScript))
            .context("failed to build control block")?;

        let reveal_vout = u32::try_from(reveal_output).context("reveal vout exceeds u32")?;
        let mut reveal_tx =
            build_reveal_transaction_template(commit_tx, reveal_vout, recipient, &self.tag_script);

        // Set output value so the fee tracks the requested fee rate for this witness shape.
        let vsize = estimate_reveal_vsize(reveal_tx.clone(), &self.reveal_script, &control_block);
        let fee = vsize as u64 * fee_rate;
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
            &self.reveal_script,
            &self.taproot_spend_info,
            &self.keypair,
        )?;
        Ok(reveal_tx)
    }
}

/// Funds and signs the commit tx with the hardcoded regtest test wallet (functional tests).
fn fund_commit_with_test_wallet(
    args: &PredicateUpdateRequest,
    reveal_address: &Address,
    fee_rate_sat_per_vb: u32,
) -> anyhow::Result<(Transaction, Address)> {
    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        &args.btc_url,
        None,
        Some(&args.btc_user),
        Some(&args.btc_password),
    )?;
    sync_wallet(&mut wallet, &client)?;

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

    let finalized = wallet
        .sign(&mut psbt, Default::default())
        .context("failed to sign commit tx")?;
    ensure!(finalized, "test wallet could not finalize commit tx");

    let commit_tx = psbt.extract_tx().context("failed to finalize commit tx")?;
    let recipient = wallet.peek_address(KeychainKind::External, 0).address;
    Ok((commit_tx, recipient))
}

/// Funds and signs the commit tx with a `bitcoind` wallet
/// (`createrawtransaction` -> `fundrawtransaction` -> `signrawtransactionwithwallet`).
/// Nothing is broadcast and no UTXOs are locked.
fn fund_commit_with_bitcoind_wallet(
    args: &PredicateUpdateRequest,
    wallet: &str,
    reveal_address: &Address,
) -> anyhow::Result<(Transaction, Address)> {
    let url = format!("{}/wallet/{wallet}", args.btc_url.trim_end_matches('/'));
    let client = Client::new(
        &url,
        Auth::UserPass(args.btc_user.clone(), args.btc_password.clone()),
    )
    .context("failed to create bitcoind wallet RPC client")?;

    let chain = client
        .get_blockchain_info()
        .context("getblockchaininfo failed")?
        .chain;
    ensure!(
        chain == args.network,
        "bitcoind is on {chain}, but --network is {}",
        args.network
    );

    let outs = HashMap::from([(
        reveal_address.to_string(),
        Amount::from_sat(args.commit_output_sats),
    )]);
    let raw = client
        .create_raw_transaction_hex(&[], &outs, None, Some(true))
        .context("createrawtransaction failed")?;

    // `feeRate` is BTC per kvB: sat/vB * 1000 = sat/kvB.
    let fee_rate_per_kvb = args
        .fee_rate
        .checked_mul(1000)
        .context("fee rate overflow")?;
    let options = FundRawTransactionOptions {
        fee_rate: Some(Amount::from_sat(fee_rate_per_kvb)),
        replaceable: Some(true),
        ..Default::default()
    };
    // Zero-input txs are ambiguous with the segwit marker; createrawtransaction emits legacy.
    let funded = client
        .fund_raw_transaction(raw, Some(&options), Some(false))
        .context("fundrawtransaction failed")?;

    let signed = client
        .sign_raw_transaction_with_wallet(&funded.hex, None, None)
        .context("signrawtransactionwithwallet failed")?;
    if !signed.complete {
        bail!(
            "wallet could not fully sign commit tx: {:?}",
            signed.errors.unwrap_or_default()
        );
    }
    let commit_tx = signed
        .transaction()
        .context("failed to decode signed commit tx")?;

    let recipient = client
        .get_raw_change_address(None)
        .context("getrawchangeaddress failed")?
        .require_network(args.network)
        .context("wallet change address is on the wrong network")?;

    Ok((commit_tx, recipient))
}

fn random_secret_key() -> anyhow::Result<SecretKey> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    SecretKey::from_slice(&bytes).context("failed to generate envelope key")
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

fn generate_keypair(secret_key: SecretKey) -> anyhow::Result<(UntweakedKeypair, XOnlyPublicKey)> {
    let keypair = UntweakedKeypair::from_seckey_slice(SECP256K1, &secret_key.secret_bytes())
        .context("failed to create keypair")?;
    let xonly = XOnlyPublicKey::from_keypair(&keypair).0;
    Ok((keypair, xonly))
}

fn build_reveal_script_and_address(
    envelope_bytes: &[u8],
    xonly_pubkey: XOnlyPublicKey,
    network: Network,
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
        network,
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

    use bdk_wallet::bitcoin::secp256k1::PublicKey;
    use strata_asm_admin_threshold_sig::{
        verify_threshold_signatures, IndexedSignature, P2wpkhAddress,
    };
    use strata_asm_proto_admin_txs::test_utils::sign_ecdsa_bip137;

    use super::*;

    /// Builds the real reveal tx (on a stub commit tx, no RPC) for signet with a 2-of-3
    /// signature set and checks ASM's own parser recovers a payload that passes the
    /// on-chain threshold check.
    #[test]
    fn signet_reveal_tx_decodes_and_verifies() {
        let network = Network::Signet;
        let magic = MagicBytes::new(*b"ALPN");
        let keys = [1u8, 2, 3].map(|b| SecretKey::from_slice(&[b; 32]).expect("key"));
        let signers = keys
            .iter()
            .map(|k| P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, k)))
            .collect();
        let config = ThresholdConfig::try_new(signers, NonZero::new(2).expect("nz")).expect("cfg");

        for target in [
            PredicateUpdateTarget::EeStfVk,
            PredicateUpdateTarget::OlStfVk,
        ] {
            let action = build_predicate_update_action(PredicateKey::always_accept(), target);
            let (_, digest) = signing_message(&action, 5, network);
            let sigs = vec![
                IndexedSignature::new(0, sign_ecdsa_bip137(&digest, &keys[0])),
                IndexedSignature::new(2, sign_ecdsa_bip137(&digest, &keys[2])),
            ];
            let payload = SignedPayload::new(5, action, SignatureSet::new(sigs).expect("set"));

            let reveal = RevealParts::new(&payload, magic, network).expect("reveal parts");
            let commit_tx = Transaction {
                version: Version(2),
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![TxOut {
                    value: Amount::from_sat(20_000),
                    script_pubkey: reveal.address.script_pubkey(),
                }],
            };
            let recipient = reveal.address.clone();
            let reveal_tx = reveal
                .build_signed_reveal_tx(&commit_tx, recipient, 2)
                .expect("reveal tx");

            let decoded = decode_reveal_tx(&reveal_tx, magic, &payload).expect("decodes");
            let (_, chain_digest) = signing_message(&decoded.action, decoded.seqno, network);
            verify_threshold_signatures(&config, decoded.signatures.signatures(), &chain_digest)
                .expect("threshold ok");

            // wrong magic must not decode
            assert!(decode_reveal_tx(&reveal_tx, MagicBytes::new(*b"XXXX"), &payload).is_err());
        }
    }
}

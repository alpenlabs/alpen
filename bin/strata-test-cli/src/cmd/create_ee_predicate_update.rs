//! CLI commands for creating and broadcasting predicate admin updates.
//!
//! Supports M-of-N threshold signing (local keys and/or externally produced BIP-137
//! signatures), any network/magic, a local dry-run of the on-chain threshold check against
//! the deployed signer config, and funding from a `bitcoind` wallet.

use std::{path::PathBuf, slice, str::FromStr};

use anyhow::{bail, ensure, Context};
use argh::FromArgs;
use bdk_wallet::bitcoin::{
    absolute::LockTime,
    blockdata::script,
    consensus::serialize,
    key::UntweakedKeypair,
    secp256k1::{schnorr::Signature, Message, SecretKey, XOnlyPublicKey, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use rand::RngCore;
use serde_json::{json, Value};
use strata_asm_admin_threshold_sig::{SignatureSet, ThresholdConfig};
use strata_asm_admin_types::UpdateTxType;
use strata_asm_proto_admin_txs::{
    actions::{
        updates::{AsmStfVkUpdate, EeStfVkUpdate, OlStfVkUpdate},
        CancelAction, MultisigAction, UpdateAction, UpdateId,
    },
    parser::SignedPayload,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_envelope_fmt::EnvelopeScriptBuilder;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_predicate::{PredicateKey, PredicateTypeId};

use super::admin_action::{prepare_message, write_message};
use crate::{
    admin::{
        assemble_signatures, decode_reveal_tx, parse_admin_key, parse_external_signature,
        parse_secret_key, signing_message, threshold_config_from_addresses,
        threshold_config_from_asm_params, verify_signatures,
    },
    admin_bitcoin::{BitcoinFunding, BitcoinFundingConfig},
    constants::{MAGIC_BYTES, NETWORK},
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

/// Broadcast an ASM STF predicate update (role: StrataAdministrator).
///
/// Every ASM executor must already list the new predicate in its execution targets: one
/// that does not halts at the first block after the update activates.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-asm-predicate-update")]
pub struct CreateAsmPredicateUpdateArgs {
    /// update sequence number; must be > the role's last accepted seqno (max gap per params)
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `Sp1Groth16:<hex>`)
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

/// Broadcast a cancel of a queued predicate update (role: that of the cancelled update).
///
/// The chain accepts the cancel only if `--update` and `--predicate` match the queued
/// update exactly and it lands before the update activates.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-admin-cancel")]
pub struct CreateAdminCancelArgs {
    /// ID of the queued update to cancel (the admin state's `next_update_id` when it queued)
    #[argh(option)]
    pub target_id: UpdateId,

    /// kind of the queued update: `ee-stf-vk`, `ol-stf-vk` or `asm-stf-vk`
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,

    /// predicate of the queued update
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,

    /// cancel sequence number; must be > the role's last accepted seqno (max gap per params)
    #[argh(option)]
    pub seq_no: u64,

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
    /// update kind: `ee-stf-vk` (AlpenAdministrator), `ol-stf-vk` or `asm-stf-vk`
    /// (StrataAdministrator)
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,

    /// update sequence number
    #[argh(option)]
    pub seq_no: u64,

    /// target predicate (e.g. `Sp1Groth16:<hex>`)
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: Option<PredicateKey>,

    /// SHA-256 of the predicate CONDITION bytes, not the predicate file (requires
    /// --predicate-type)
    #[argh(option, from_str_fn(parse_predicate_hash))]
    pub predicate_hash: Option<[u8; 32]>,

    /// predicate type for hash-only signing; currently Sp1Groth16 (>32-byte conditions)
    #[argh(option, from_str_fn(parse_predicate_type))]
    pub predicate_type: Option<PredicateTypeId>,

    /// bitcoin network the chain is anchored to (default regtest)
    #[argh(option, default = "NETWORK", from_str_fn(parse_network))]
    pub network: Network,

    /// print the message for cancelling queued update `<id>` instead of the update itself
    #[argh(option)]
    pub cancel_target_id: Option<UpdateId>,
}

pub(crate) fn parse_predicate_key(value: &str) -> Result<PredicateKey, String> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|e| e.to_string())
}

pub(crate) fn parse_network(value: &str) -> Result<Network, String> {
    Network::from_str(value).map_err(|e| e.to_string())
}

fn parse_magic(value: &str) -> Result<MagicBytes, String> {
    if !value.is_ascii() {
        return Err(format!("magic `{value}` must be ASCII"));
    }
    MagicBytes::from_str(value).map_err(|e| e.to_string())
}

pub(crate) fn parse_target(value: &str) -> Result<PredicateUpdateTarget, String> {
    match value {
        "ee-stf-vk" => Ok(PredicateUpdateTarget::EeStfVk),
        "ol-stf-vk" => Ok(PredicateUpdateTarget::OlStfVk),
        "asm-stf-vk" => Ok(PredicateUpdateTarget::AsmStfVk),
        other => Err(format!(
            "unknown update `{other}` (expected ee-stf-vk|ol-stf-vk|asm-stf-vk)"
        )),
    }
}

/// Minimum non-dust value for the reveal output.
const MIN_REVEAL_OUTPUT_SATS: u64 = 546;

/// Which predicate an admin update targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredicateUpdateTarget {
    /// EE STF verifying key (AlpenAdministrator).
    EeStfVk,
    /// OL STF / checkpoint verifying key (StrataAdministrator).
    OlStfVk,
    /// ASM STF verifying key, which selects the ASM spec (StrataAdministrator).
    AsmStfVk,
}

impl PredicateUpdateTarget {
    pub(crate) fn update_type(self) -> UpdateTxType {
        match self {
            Self::EeStfVk => UpdateTxType::EeStfVkUpdate,
            Self::OlStfVk => UpdateTxType::OlStfVkUpdate,
            Self::AsmStfVk => UpdateTxType::AsmStfVkUpdate,
        }
    }
}

pub(crate) fn parse_predicate_hash(value: &str) -> Result<[u8; 32], String> {
    let bytes =
        hex::decode(value).map_err(|e| format!("predicate hash must be 64 hex characters: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "predicate hash must be exactly 32 bytes (64 hex characters)".to_owned())
}

pub(crate) fn parse_predicate_type(value: &str) -> Result<PredicateTypeId, String> {
    PredicateTypeId::from_str(value).map_err(|e| e.to_string())
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
    target: PredicateUpdateTarget,
    /// Cancel the queued update with this ID instead of submitting the update.
    cancel_target_id: Option<UpdateId>,
}

pub(crate) fn create_ee_predicate_update(
    args: CreateEePredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let bitcoin = BitcoinFundingConfig {
        rpc_url: args.btc_url,
        rpc_user: args.btc_user,
        rpc_password: args.btc_password,
        wallet: args.btc_wallet,
        fee_rate_sat_vb: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
    };
    create_predicate_update(
        PredicateUpdateRequest {
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
            target: PredicateUpdateTarget::EeStfVk,
            cancel_target_id: None,
        },
        bitcoin,
        args.dry_run,
    )
}

pub(crate) fn create_checkpoint_predicate_update(
    args: CreateCheckpointPredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let bitcoin = BitcoinFundingConfig {
        rpc_url: args.btc_url,
        rpc_user: args.btc_user,
        rpc_password: args.btc_password,
        wallet: args.btc_wallet,
        fee_rate_sat_vb: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
    };
    create_predicate_update(
        PredicateUpdateRequest {
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
            target: PredicateUpdateTarget::OlStfVk,
            cancel_target_id: None,
        },
        bitcoin,
        args.dry_run,
    )
}

pub(crate) fn create_asm_predicate_update(
    args: CreateAsmPredicateUpdateArgs,
) -> Result<(), DisplayedError> {
    let bitcoin = BitcoinFundingConfig {
        rpc_url: args.btc_url,
        rpc_user: args.btc_user,
        rpc_password: args.btc_password,
        wallet: args.btc_wallet,
        fee_rate_sat_vb: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
    };
    create_predicate_update(
        PredicateUpdateRequest {
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
            target: PredicateUpdateTarget::AsmStfVk,
            cancel_target_id: None,
        },
        bitcoin,
        args.dry_run,
    )
}

pub(crate) fn create_admin_cancel(args: CreateAdminCancelArgs) -> Result<(), DisplayedError> {
    let bitcoin = BitcoinFundingConfig {
        rpc_url: args.btc_url,
        rpc_user: args.btc_user,
        rpc_password: args.btc_password,
        wallet: args.btc_wallet,
        fee_rate_sat_vb: args.fee_rate,
        commit_output_sats: args.commit_output_sats,
    };
    create_predicate_update(
        PredicateUpdateRequest {
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
            target: args.update,
            cancel_target_id: Some(args.target_id),
        },
        bitcoin,
        args.dry_run,
    )
}

pub(crate) fn print_signing_message(args: PrintSigningMessageArgs) -> Result<(), DisplayedError> {
    let message = prepare_message(
        args.update,
        args.seq_no,
        args.network,
        args.cancel_target_id,
        args.predicate,
        args.predicate_hash,
        args.predicate_type,
    )
    .user_error("invalid admin signing request")?;
    write_message(&message).internal_error("failed to write admin signing message")
}

fn create_predicate_update(
    args: PredicateUpdateRequest,
    bitcoin: BitcoinFundingConfig,
    dry_run: bool,
) -> Result<(), DisplayedError> {
    let signed = sign_and_verify(&args).user_error("admin signature check failed")?;
    let bitcoin = BitcoinFunding::connect(bitcoin, args.network)
        .user_error("invalid Bitcoin funding configuration")?;
    let (commit_tx, reveal_tx) = build_admin_commit_reveal_pair(&bitcoin, args.magic, &signed)
        .internal_error("failed to build admin tx pair")?;

    let decoded = decode_reveal_tx(&reveal_tx, args.magic, &signed.payload)
        .internal_error("built reveal tx does not decode to the signed payload")?;

    if dry_run {
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

    let commit_txid = bitcoin
        .broadcast_tx(&commit_tx)
        .internal_error("failed to broadcast commit tx")?;
    let reveal_txid = bitcoin
        .broadcast_reveal_with_retry(&reveal_tx)
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
    let action = build_admin_action(args.predicate.clone(), args.target, args.cancel_target_id);
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
    bitcoin: &BitcoinFunding,
    magic: MagicBytes,
    signed: &SignedAdminPayload,
) -> anyhow::Result<(Transaction, Transaction)> {
    if bitcoin.commit_output_sats() <= MIN_REVEAL_OUTPUT_SATS {
        bail!("commit_output_sats must be > {MIN_REVEAL_OUTPUT_SATS}");
    }
    let reveal = RevealParts::new(&signed.payload, magic, bitcoin.network())?;
    let (commit_tx, recipient) = bitcoin.fund_commit(&reveal.address)?;
    let reveal_tx =
        reveal.build_signed_reveal_tx(&commit_tx, recipient, bitcoin.fee_rate_sat_vb())?;
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

fn random_secret_key() -> anyhow::Result<SecretKey> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    SecretKey::from_slice(&bytes).context("failed to generate envelope key")
}

fn build_update_action(key: PredicateKey, target: PredicateUpdateTarget) -> UpdateAction {
    match target {
        PredicateUpdateTarget::EeStfVk => UpdateAction::EeStfVk(EeStfVkUpdate::new(key)),
        PredicateUpdateTarget::OlStfVk => UpdateAction::OlStfVk(OlStfVkUpdate::new(key)),
        PredicateUpdateTarget::AsmStfVk => UpdateAction::AsmStfVk(AsmStfVkUpdate::new(key)),
    }
}

/// Builds the predicate update, or a cancel of it when `cancel_target_id` names the queued
/// update. A cancel embeds the update so its role and signing message match the update's.
pub(crate) fn build_admin_action(
    key: PredicateKey,
    target: PredicateUpdateTarget,
    cancel_target_id: Option<UpdateId>,
) -> MultisigAction {
    let update = build_update_action(key, target);
    match cancel_target_id {
        Some(target_id) => MultisigAction::Cancel(CancelAction::new(target_id, update)),
        None => MultisigAction::Update(update),
    }
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

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use bdk_wallet::bitcoin::secp256k1::PublicKey;
    use strata_asm_admin_threshold_sig::{
        verify_threshold_signatures, IndexedSignature, P2wpkhAddress,
    };
    use strata_asm_admin_types::Role;
    use strata_asm_proto_admin_txs::test_utils::sign_ecdsa_bip137;
    use strata_crypto::hash;

    use super::*;
    use crate::admin::predicate_hash_signing_message;

    #[test]
    fn authorization_request_requires_no_bitcoin_funding_configuration() {
        let keys = [1u8, 2].map(|b| SecretKey::from_slice(&[b; 32]).expect("synthetic key"));
        let request = PredicateUpdateRequest {
            seq_no: 5,
            predicate: PredicateKey::always_accept(),
            admin_xpriv: None,
            admin_key: keys
                .iter()
                .enumerate()
                .map(|(i, key)| format!("{i}:{}", hex::encode(key.secret_bytes())))
                .collect(),
            signature: vec![],
            signers_config: None,
            threshold: Some(2),
            signer_address: keys
                .iter()
                .map(|key| {
                    P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, key))
                        .to_address(Network::Signet)
                        .to_string()
                })
                .collect(),
            network: Network::Signet,
            magic: MagicBytes::new(*b"ALPN"),
            target: PredicateUpdateTarget::EeStfVk,
            cancel_target_id: None,
        };
        let signed = sign_and_verify(&request).expect("authorization is entirely offline");
        assert!(signed.verified);
        assert_eq!(signed.payload.seqno, 5);
        assert_eq!(
            signed.payload.action.required_role(),
            Role::AlpenAdministrator
        );
    }

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

        let targets = [
            PredicateUpdateTarget::EeStfVk,
            PredicateUpdateTarget::OlStfVk,
            PredicateUpdateTarget::AsmStfVk,
        ];
        for (target, cancel_target_id) in
            targets.into_iter().flat_map(|t| [(t, None), (t, Some(2))])
        {
            let predicate = PredicateKey::try_new(PredicateTypeId::Sp1Groth16, vec![0x42; 64])
                .expect("test predicate");
            let predicate_hash = hash::raw(predicate.condition()).0;
            let action = build_admin_action(predicate, target, cancel_target_id);
            let (canonical, canonical_digest) = signing_message(&action, 5, network);
            let (text, digest) = predicate_hash_signing_message(
                target.update_type(),
                cancel_target_id,
                PredicateTypeId::Sp1Groth16,
                predicate_hash,
                5,
                network,
            )
            .expect("hash-based message");
            assert_eq!(text, canonical.as_str());
            assert_eq!(digest, canonical_digest);
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

    /// The ASM STF update and its cancel are Strata Administrator actions, and the cancel
    /// message names the queued update it targets.
    #[test]
    fn asm_stf_vk_update_and_cancel_messages() {
        let target = parse_target("asm-stf-vk").expect("known target");
        let key = PredicateKey::always_accept();

        let update = build_admin_action(key.clone(), target, None);
        assert_eq!(update.required_role(), Role::StrataAdministrator);
        let (message, _) = signing_message(&update, 2, Network::Signet);
        let text = message.as_str();
        assert!(text.contains("Network: signet"), "{text}");
        assert!(text.contains("Action: ASM STF VK Update"), "{text}");
        assert!(
            text.contains("Authorized By: Strata Administrator"),
            "{text}"
        );
        assert!(text.contains("Sequence: 2"), "{text}");

        let cancel = build_admin_action(key, target, Some(2));
        assert_eq!(cancel.required_role(), Role::StrataAdministrator);
        let (message, _) = signing_message(&cancel, 3, Network::Signet);
        let text = message.as_str();
        assert!(text.contains("Action: Cancel"), "{text}");
        assert!(text.contains("Sequence: 3"), "{text}");
        assert!(text.contains("Target Id: 2"), "{text}");
        assert!(text.contains("Target Update: ASM STF VK Update"), "{text}");
    }
}

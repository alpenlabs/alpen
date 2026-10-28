//! Sign a predicate commitment offline; transaction construction remains separate.

use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};

use anyhow::{ensure, Context};
use argh::FromArgs;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bitcoin::{
    secp256k1::{PublicKey, SECP256K1},
    Network,
};
use strata_asm_admin_threshold_sig::P2wpkhAddress;
use strata_asm_params::AsmParams;
use strata_asm_proto_admin_txs::test_utils::sign_ecdsa_bip137;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_predicate::PredicateTypeId;

use super::create_ee_predicate_update::{
    parse_network, parse_predicate_hash, parse_predicate_type, parse_target, PredicateUpdateTarget,
};
use crate::admin::{
    parse_secret_key, predicate_hash_signing_message, threshold_config_from_asm_params_json,
};

/// Sign an admin predicate commitment; read the key from stdin and emit <index>:<base64>.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "sign-admin-action")]
pub struct SignAdminActionArgs {
    /// update kind: ee-stf-vk, ol-stf-vk or asm-stf-vk
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,
    /// current valid authority sequence number
    #[argh(option)]
    pub seq_no: u64,
    /// SHA-256 of predicate condition bytes, not the predicate file
    #[argh(option, from_str_fn(parse_predicate_hash))]
    pub predicate_hash: [u8; 32],
    /// predicate type (Sp1Groth16 with a condition longer than 32 bytes)
    #[argh(option, from_str_fn(parse_predicate_type))]
    pub predicate_type: PredicateTypeId,
    /// bitcoin network, required explicitly
    #[argh(option, from_str_fn(parse_network))]
    pub network: Network,
    /// cancel this queued update instead of authorizing the update
    #[argh(option)]
    pub cancel_target_id: Option<u32>,
    /// trusted ASM params supplying the authority's ordered signer set
    #[argh(option)]
    pub signers_config: PathBuf,
    /// read an xpriv or 32-byte hex secret from stdin, never argv
    #[argh(switch)]
    pub key_stdin: bool,
}

pub(crate) fn sign_admin_action(args: SignAdminActionArgs) -> Result<(), DisplayedError> {
    sign(args).user_error("offline admin signing failed")
}

fn sign(args: SignAdminActionArgs) -> anyhow::Result<()> {
    ensure!(
        args.key_stdin,
        "provide the signing key through --key-stdin"
    );
    let (text, digest) = predicate_hash_signing_message(
        args.update.update_type(),
        args.cancel_target_id,
        args.predicate_type,
        args.predicate_hash,
        args.seq_no,
        args.network,
    )?;
    let raw = fs::read_to_string(&args.signers_config).context("read signer configuration")?;
    let params: AsmParams = serde_json::from_str(&raw).context("parse ASM params")?;
    let config = threshold_config_from_asm_params_json(
        &raw,
        args.update.update_type().authorized_role(),
        args.network,
        params.magic,
    )?;
    writeln!(io::stderr().lock(), "{text}").context("display action to sign")?;
    let mut input = String::new();
    io::stdin()
        .take(4097)
        .read_to_string(&mut input)
        .context("read signing key")?;
    ensure!(
        input.len() <= 4096,
        "signing key exceeds maximum input size"
    );
    let key = parse_secret_key(input.trim())?;
    let address = P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, &key));
    let index = config
        .signers()
        .iter()
        .position(|signer| *signer == address)
        .context("key is not a signer for this action's authority")?;
    let signature = BASE64.encode(sign_ecdsa_bip137(&digest, &key));
    writeln!(io::stdout().lock(), "{index}:{signature}").context("write signature")
}

//! Offline admin message signing and signature verification; no funding or Bitcoin RPC.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    num::NonZero,
    path::{Path, PathBuf},
    slice,
};

use anyhow::{bail, ensure, Context};
use argh::FromArgs;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bitcoin::{
    secp256k1::{PublicKey, SecretKey, SECP256K1},
    Network,
};
use serde_json::{json, Value};
use strata_asm_admin_threshold_sig::{
    verify_threshold_signatures, IndexedSignature, P2wpkhAddress, ThresholdConfig,
    ThresholdSignatureError,
};
use strata_asm_admin_types::Role;
use strata_asm_params::AsmParams;
use strata_asm_proto_admin_txs::test_utils::sign_ecdsa_bip137;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_predicate::{PredicateKey, PredicateTypeId};

use super::create_ee_predicate_update::{
    build_admin_action, parse_network, parse_predicate_hash, parse_predicate_key,
    parse_predicate_type, parse_target, PredicateUpdateTarget,
};
use crate::admin::{
    assemble_signatures, parse_external_signature, parse_secret_key,
    predicate_hash_signing_message, signing_message, threshold_config_from_asm_params_json,
    verify_signatures,
};

/// Sign one admin authorization offline; emits <index>:<base64> for the existing tx commands.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "sign-admin-action")]
pub struct SignAdminActionArgs {
    /// update kind: ee-stf-vk, ol-stf-vk or asm-stf-vk
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,

    /// sequence number selected from the current authority state
    #[argh(option)]
    pub seq_no: u64,

    /// complete predicate; mutually exclusive with --predicate-hash
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: Option<PredicateKey>,

    /// SHA-256 of the predicate condition (>32 bytes), not the predicate file
    #[argh(option, from_str_fn(parse_predicate_hash))]
    pub predicate_hash: Option<[u8; 32]>,

    /// type accompanying --predicate-hash (currently Sp1Groth16)
    #[argh(option, from_str_fn(parse_predicate_type))]
    pub predicate_type: Option<PredicateTypeId>,

    /// bitcoin network, required explicitly
    #[argh(option, from_str_fn(parse_network))]
    pub network: Network,

    /// cancel this queued update ID instead of authorizing the update
    #[argh(option)]
    pub cancel_target_id: Option<u32>,

    /// trusted ASM params JSON supplying the required role's ordered signer configuration
    #[argh(option)]
    pub signers_config: PathBuf,

    /// read the signing secret (xpriv or 32-byte hex) from stdin, never argv
    #[argh(switch)]
    pub key_stdin: bool,

    /// private file containing an xpriv or 32-byte hex secret; alternative to --key-stdin
    #[argh(option)]
    pub key_file: Option<PathBuf>,
}

/// Check collected authorizations offline, before any transaction construction or funding.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "verify-admin-signatures")]
pub struct VerifyAdminSignaturesArgs {
    /// update kind: ee-stf-vk, ol-stf-vk or asm-stf-vk
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,

    /// sequence number selected from the current authority state
    #[argh(option)]
    pub seq_no: u64,

    /// complete predicate; mutually exclusive with --predicate-hash
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: Option<PredicateKey>,

    /// SHA-256 of the predicate condition (>32 bytes), not the predicate file
    #[argh(option, from_str_fn(parse_predicate_hash))]
    pub predicate_hash: Option<[u8; 32]>,

    /// type accompanying --predicate-hash (currently Sp1Groth16)
    #[argh(option, from_str_fn(parse_predicate_type))]
    pub predicate_type: Option<PredicateTypeId>,

    /// bitcoin network, required explicitly
    #[argh(option, from_str_fn(parse_network))]
    pub network: Network,

    /// cancel this queued update ID instead of authorizing the update
    #[argh(option)]
    pub cancel_target_id: Option<u32>,

    /// trusted ASM params JSON supplying the required role's ordered signer configuration
    #[argh(option)]
    pub signers_config: PathBuf,

    /// base64 wallet signature or <index>:<base64>, repeatable; bare signatures are indexed
    /// automatically
    #[argh(option)]
    pub signature: Vec<String>,

    /// report an incomplete quorum without failing; invalid/duplicate signatures still fail
    #[argh(switch)]
    pub allow_partial: bool,
}

/// Validated canonical message and its derived authority, independent of Bitcoin funding.
#[derive(Debug)]
pub(crate) struct AdminMessage {
    text: String,
    digest: [u8; 32],
    role: Role,
    network: Network,
}

pub(crate) fn prepare_message(
    update: PredicateUpdateTarget,
    seqno: u64,
    network: Network,
    cancel_target_id: Option<u32>,
    predicate: Option<PredicateKey>,
    predicate_hash: Option<[u8; 32]>,
    predicate_type: Option<PredicateTypeId>,
) -> anyhow::Result<AdminMessage> {
    let role = update.update_type().authorized_role();
    let (text, digest) = match (predicate, predicate_hash, predicate_type) {
        (Some(predicate), None, None) => {
            let action = build_admin_action(predicate, update, cancel_target_id);
            let (message, digest) = signing_message(&action, seqno, network);
            (message.as_str().to_owned(), digest)
        }
        (None, Some(hash), Some(kind)) => predicate_hash_signing_message(
            update.update_type(),
            cancel_target_id,
            kind,
            hash,
            seqno,
            network,
        )?,
        _ => bail!(
            "provide either --predicate alone, or --predicate-hash together with --predicate-type"
        ),
    };
    Ok(AdminMessage {
        text,
        digest,
        role,
        network,
    })
}

pub(crate) fn write_message(message: &AdminMessage) -> anyhow::Result<()> {
    io::stdout()
        .lock()
        .write_all(message.text.as_bytes())
        .context("write exact signing text")?;
    write_review(message)
}

fn write_review(message: &AdminMessage) -> anyhow::Result<()> {
    writeln!(
        io::stderr().lock(),
        "\nrole: {}\nnetwork: {}\nsignmessage digest (cross-check only, do not sign): {}",
        message.role,
        message.network,
        hex::encode(message.digest)
    )
    .context("write signing context")
}

fn load_config(path: &Path, message: &AdminMessage) -> anyhow::Result<ThresholdConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read signer config {}", path.display()))?;
    let params: AsmParams = serde_json::from_str(&raw).context("parse ASM params")?;
    threshold_config_from_asm_params_json(&raw, message.role, message.network, params.magic)
}

fn read_key(key_stdin: bool, key_file: Option<&Path>) -> anyhow::Result<SecretKey> {
    ensure!(
        key_stdin != key_file.is_some(),
        "choose exactly one of --key-stdin or --key-file"
    );
    let mut value = String::new();
    match key_file {
        Some(path) => {
            File::open(path)
                .context("open signing-key file")?
                .take(4097)
                .read_to_string(&mut value)
                .context("read signing-key file")?;
        }
        None => {
            io::stdin()
                .take(4097)
                .read_to_string(&mut value)
                .context("read signing key from stdin")?;
        }
    }
    ensure!(
        value.len() <= 4096,
        "signing key exceeds maximum input size"
    );
    parse_secret_key(value.trim()).context("invalid signing key")
}

fn sign_one(
    message: &AdminMessage,
    config: &ThresholdConfig,
    key: &SecretKey,
) -> anyhow::Result<String> {
    let signer = P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, key));
    let index = config
        .signers()
        .iter()
        .position(|expected| *expected == signer)
        .context("key is not a signer for this admin action's authority")?;
    let index = u8::try_from(index).context("signer index exceeds u8")?;
    let signature = sign_ecdsa_bip137(&message.digest, key);
    let result = format!("{index}:{}", BASE64.encode(signature));
    let indexed = parse_external_signature(&result)?;
    let individual = ThresholdConfig::try_new(config.signers().to_vec(), NonZero::<u8>::MIN)?;
    verify_signatures(&individual, &[indexed], &message.digest, message.network)?;
    Ok(result)
}

pub(crate) fn sign_admin_action(args: SignAdminActionArgs) -> Result<(), DisplayedError> {
    let message = prepare_message(
        args.update,
        args.seq_no,
        args.network,
        args.cancel_target_id,
        args.predicate,
        args.predicate_hash,
        args.predicate_type,
    )
    .user_error("invalid signing request")?;
    let config =
        load_config(&args.signers_config, &message).user_error("invalid signer configuration")?;
    writeln!(io::stderr().lock(), "{}", message.text).internal_error("write action for review")?;
    write_review(&message).internal_error("write signing context")?;
    let key =
        read_key(args.key_stdin, args.key_file.as_deref()).user_error("cannot load signing key")?;
    let signature = sign_one(&message, &config, &key).user_error("admin signing failed")?;
    writeln!(io::stdout().lock(), "{signature}").internal_error("write signature")
}

/// Resolve bare wallet signatures with the protocol verifier, avoiding a second recovery
/// implementation. An invalid signature for a candidate index means only "not this signer";
/// malformed-signature and other structural errors propagate immediately.
fn resolve_signature(
    spec: &str,
    config: &ThresholdConfig,
    digest: &[u8; 32],
) -> anyhow::Result<IndexedSignature> {
    if spec.contains(':') {
        return parse_external_signature(spec);
    }
    let parsed = parse_external_signature(&format!("0:{spec}"))?;
    let mut raw = [0u8; 65];
    raw[0] = parsed.recovery_id();
    raw[1..].copy_from_slice(&parsed.compact());
    let individual = ThresholdConfig::try_new(config.signers().to_vec(), NonZero::<u8>::MIN)?;
    for index in 0..config.signers().len() {
        let candidate = IndexedSignature::new(u8::try_from(index)?, raw);
        match verify_threshold_signatures(&individual, slice::from_ref(&candidate), digest) {
            Ok(()) => return Ok(candidate),
            Err(ThresholdSignatureError::InvalidSignature { .. }) => continue,
            Err(error) => return Err(error).context("invalid external signature"),
        }
    }
    bail!("signature does not match any signer for this action and message")
}

fn indexed_signature_text(signature: &IndexedSignature) -> String {
    let mut raw = [0u8; 65];
    raw[0] = signature.recovery_id();
    raw[1..].copy_from_slice(&signature.compact());
    format!("{}:{}", signature.index(), BASE64.encode(raw))
}

fn verify_collected(
    message: &AdminMessage,
    config: &ThresholdConfig,
    signatures: &[String],
    allow_partial: bool,
) -> anyhow::Result<Value> {
    let external = signatures
        .iter()
        .map(|s| resolve_signature(s, config, &message.digest))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let sigs = assemble_signatures(&[], external, &message.digest)?;
    let partial_config = ThresholdConfig::try_new(config.signers().to_vec(), NonZero::<u8>::MIN)?;
    verify_signatures(
        if allow_partial {
            &partial_config
        } else {
            config
        },
        &sigs,
        &message.digest,
        message.network,
    )?;
    Ok(
        json!({"role":message.role.to_string(),"network":message.network.to_string(),"signing_message":message.text,"signing_digest":hex::encode(message.digest),"valid_signer_indices":sigs.iter().map(|s|s.index()).collect::<Vec<_>>(),"signatures":sigs.iter().map(indexed_signature_text).collect::<Vec<_>>(),"valid_signatures":sigs.len(),"required_signatures":config.threshold(),"threshold_verified":sigs.len() >= usize::from(config.threshold())}),
    )
}

pub(crate) fn verify_admin_signatures(
    args: VerifyAdminSignaturesArgs,
) -> Result<(), DisplayedError> {
    let message = prepare_message(
        args.update,
        args.seq_no,
        args.network,
        args.cancel_target_id,
        args.predicate,
        args.predicate_hash,
        args.predicate_type,
    )
    .user_error("invalid verification request")?;
    let config =
        load_config(&args.signers_config, &message).user_error("invalid signer configuration")?;
    let report = verify_collected(&message, &config, &args.signature, args.allow_partial)
        .user_error("admin signature verification failed")?;
    let report =
        serde_json::to_string_pretty(&report).internal_error("serialize verification result")?;
    writeln!(io::stdout().lock(), "{report}").internal_error("write verification result")
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use strata_asm_admin_threshold_sig::{P2wpkhAddress, ThresholdConfig};
    use strata_crypto::hash;

    use super::*;
    use crate::{
        admin::{
            assemble_signatures, parse_external_signature, signing_message, verify_signatures,
        },
        cmd::{Commands, TopLevel},
    };

    fn keys() -> [SecretKey; 3] {
        [1u8, 2, 3].map(|b| SecretKey::from_slice(&[b; 32]).unwrap())
    }

    fn config(keys: &[SecretKey]) -> ThresholdConfig {
        ThresholdConfig::try_new(
            keys.iter()
                .map(|key| P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, key)))
                .collect(),
            NonZero::new(2).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn hash_only_messages_match_the_protocol_for_all_targets_and_cancels() {
        for target in [
            PredicateUpdateTarget::EeStfVk,
            PredicateUpdateTarget::OlStfVk,
            PredicateUpdateTarget::AsmStfVk,
        ] {
            for network in [Network::Bitcoin, Network::Signet, Network::Regtest] {
                for cancel in [None, Some(17)] {
                    for length in [33, 64, 1024] {
                        let condition = vec![0x42; length];
                        let predicate =
                            PredicateKey::try_new(PredicateTypeId::Sp1Groth16, condition.clone())
                                .unwrap();
                        let hash_message = prepare_message(
                            target,
                            42,
                            network,
                            cancel,
                            None,
                            Some(hash::raw(&condition).0),
                            Some(PredicateTypeId::Sp1Groth16),
                        )
                        .unwrap();
                        let full_message = prepare_message(
                            target,
                            42,
                            network,
                            cancel,
                            Some(predicate),
                            None,
                            None,
                        )
                        .unwrap();
                        assert_eq!(hash_message.text, full_message.text);
                        assert_eq!(hash_message.digest, full_message.digest);
                        assert!(!hash_message.text.ends_with('\n'));
                    }
                }
            }
        }
    }

    #[test]
    fn documented_ee_rehearsal_message_is_reproduced_exactly() {
        let hash = parse_predicate_hash(
            "f727840184a34825a7f587bbbfed60113b739ec228d5306237f8a4f0521adb59",
        )
        .unwrap();
        let message = prepare_message(
            PredicateUpdateTarget::EeStfVk,
            1000,
            Network::Bitcoin,
            None,
            None,
            Some(hash),
            Some(PredicateTypeId::Sp1Groth16),
        )
        .unwrap();
        assert_eq!(message.text,"Strata ASM Administration v1\nNetwork: bitcoin\nAction: EE STF VK Update\nAuthorized By: Alpen Administrator\nSequence: 1000\nAction Details:\n  Predicate Type: Sp1Groth16\n  Predicate Hash: f727840184a34825a7f587bbbfed60113b739ec228d5306237f8a4f0521adb59");
        let digest = hex::encode(message.digest);
        assert!(digest.starts_with("8a9a477a"), "{digest}");
        assert!(digest.ends_with("f8c0"), "{digest}");
    }

    #[test]
    fn independent_hash_signatures_verify_against_full_predicate_and_reject_mutation() {
        let keys = keys();
        let config = config(&keys);
        let predicate = PredicateKey::try_new(PredicateTypeId::Sp1Groth16, vec![7; 64]).unwrap();
        let message = prepare_message(
            PredicateUpdateTarget::EeStfVk,
            4,
            Network::Bitcoin,
            None,
            None,
            Some(hash::raw(predicate.condition()).0),
            Some(PredicateTypeId::Sp1Groth16),
        )
        .unwrap();
        let responses = vec![
            sign_one(&message, &config, &keys[0]).unwrap(),
            sign_one(&message, &config, &keys[2]).unwrap(),
        ];
        assert!(responses[0].starts_with("0:"));
        assert!(responses[1].starts_with("2:"));
        let bare: Vec<_> = responses
            .iter()
            .map(|s| s.split_once(':').unwrap().1.to_owned())
            .collect();
        let resolved = verify_collected(&message, &config, &bare, false).unwrap();
        assert_eq!(resolved["signatures"], json!(responses));
        assert!(verify_collected(
            &message,
            &config,
            &[responses[0].clone(), bare[0].clone()],
            true
        )
        .is_err());
        let report = verify_collected(&message, &config, &responses, false).unwrap();
        assert_eq!(report["threshold_verified"], true);
        assert_eq!(report["valid_signer_indices"], json!([0, 2]));
        assert!(verify_collected(&message, &config, &responses[..1], false).is_err());
        assert_eq!(
            verify_collected(&message, &config, &responses[..1], true).unwrap()
                ["threshold_verified"],
            false
        );
        assert!(verify_collected(
            &message,
            &config,
            &[responses[0].clone(), responses[0].clone()],
            true
        )
        .is_err());
        let wrong_key = SecretKey::from_slice(&[9; 32]).unwrap();
        assert!(sign_one(&message, &config, &wrong_key).is_err());
        let external = responses
            .iter()
            .map(|s| parse_external_signature(s).unwrap())
            .collect();
        let sigs = assemble_signatures(&[], external, &message.digest).unwrap();
        let full_action = build_admin_action(predicate, PredicateUpdateTarget::EeStfVk, None);
        let (_, full_digest) = signing_message(&full_action, 4, Network::Bitcoin);
        verify_signatures(&config, &sigs, &full_digest, Network::Bitcoin).unwrap();
        for (target, seqno, network, cancel) in [
            (PredicateUpdateTarget::OlStfVk, 4, Network::Bitcoin, None),
            (PredicateUpdateTarget::EeStfVk, 5, Network::Bitcoin, None),
            (PredicateUpdateTarget::EeStfVk, 4, Network::Signet, None),
            (PredicateUpdateTarget::EeStfVk, 4, Network::Bitcoin, Some(0)),
        ] {
            let changed = prepare_message(
                target,
                seqno,
                network,
                cancel,
                None,
                Some(hash::raw(&[7; 64]).0),
                Some(PredicateTypeId::Sp1Groth16),
            )
            .unwrap();
            assert!(verify_collected(&changed, &config, &responses, false).is_err());
        }
        let wrong_predicate = build_admin_action(
            PredicateKey::try_new(PredicateTypeId::Sp1Groth16, vec![8; 64]).unwrap(),
            PredicateUpdateTarget::EeStfVk,
            None,
        );
        let (_, wrong_digest) = signing_message(&wrong_predicate, 4, Network::Bitcoin);
        assert!(verify_signatures(&config, &sigs, &wrong_digest, Network::Bitcoin).is_err());
    }

    #[test]
    fn ambiguous_or_incomplete_hash_inputs_are_rejected() {
        let target = PredicateUpdateTarget::EeStfVk;
        assert!(prepare_message(target, 0, Network::Bitcoin, None, None, None, None).is_err());
        assert!(
            prepare_message(target, 0, Network::Bitcoin, None, None, Some([1; 32]), None).is_err()
        );
        assert!(prepare_message(
            target,
            0,
            Network::Bitcoin,
            None,
            Some(PredicateKey::always_accept()),
            Some([1; 32]),
            Some(PredicateTypeId::Sp1Groth16)
        )
        .is_err());
        assert!(prepare_message(
            target,
            0,
            Network::Bitcoin,
            None,
            None,
            Some([1; 32]),
            Some(PredicateTypeId::Bip340Schnorr)
        )
        .is_err());
        assert!(parse_predicate_hash("00").is_err());
        assert!(parse_predicate_hash(&"gg".repeat(32)).is_err());
        assert!(read_key(false, None).is_err());
        assert!(read_key(true, Some(Path::new("unused"))).is_err());
    }
    #[test]
    fn existing_full_predicate_cli_and_new_hash_cli_parse() {
        let old = TopLevel::from_args(
            &["strata-test-cli"],
            &[
                "print-signing-message",
                "--update",
                "ee-stf-vk",
                "--seq-no",
                "1",
                "--predicate",
                "AlwaysAccept",
            ],
        )
        .unwrap();
        match old.cmd {
            Commands::PrintSigningMessage(args) => {
                assert!(args.predicate.is_some());
                assert!(args.predicate_hash.is_none());
            }
            _ => panic!("expected print-signing-message"),
        }
        let hash = "11".repeat(32);
        let new = TopLevel::from_args(
            &["strata-test-cli"],
            &[
                "sign-admin-action",
                "--update",
                "ee-stf-vk",
                "--seq-no",
                "1",
                "--network",
                "bitcoin",
                "--predicate-hash",
                &hash,
                "--predicate-type",
                "Sp1Groth16",
                "--signers-config",
                "params.json",
                "--key-stdin",
            ],
        )
        .unwrap();
        match new.cmd {
            Commands::SignAdminAction(args) => {
                assert!(args.key_stdin);
                assert_eq!(args.predicate_hash, Some([0x11; 32]));
            }
            _ => panic!("expected sign-admin-action"),
        }
    }
}

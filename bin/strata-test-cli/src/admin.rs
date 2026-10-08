//! Threshold-signature helpers for ASM administration payloads.
//!
//! Everything here mirrors what the ASM admin subprotocol does on-chain
//! (`MultisigAuthority::verify_action_signature`), so a payload that passes
//! [`verify_signatures`] locally is accepted by the chain modulo sequence-number state.

use std::{fs, num::NonZero, path::Path, slice, str::FromStr};

use anyhow::{anyhow, bail, ensure, Context};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bitcoin::{
    address::NetworkUnchecked, bip32::Xpriv, secp256k1::SecretKey, Address, Network, Transaction,
};
use strata_asm_admin_threshold_sig::{
    verify_threshold_signatures, IndexedSignature, ThresholdConfig,
};
use strata_asm_admin_types::{Role, UncheckedThresholdConfig};
use strata_asm_common::TxInputRef;
use strata_asm_params::AsmParams;
use strata_asm_proto_admin_txs::{
    actions::MultisigAction,
    parser::{parse_tx, SignedPayload},
    signing_message::SigningMessage,
    test_utils::sign_ecdsa_bip137,
};
use strata_l1_txfmt::{MagicBytes, ParseConfig};

/// Length of a BIP-137 recoverable signature (`header || r || s`).
const BIP137_SIG_LEN: usize = 65;

/// Renders the canonical signing message for `action` and returns it with its
/// BIP-137 `signMessage` digest.
pub(crate) fn signing_message(
    action: &MultisigAction,
    seqno: u64,
    network: Network,
) -> (SigningMessage, [u8; 32]) {
    let message = SigningMessage::for_action(action, seqno, network);
    let digest = message.compute_sighash().0;
    (message, digest)
}

/// Splits an `<index>:<value>` spec into a signer index and the value.
fn split_indexed(spec: &str) -> anyhow::Result<(u8, &str)> {
    let (index, value) = spec
        .split_once(':')
        .ok_or_else(|| anyhow!("expected `<signer_index>:<value>`, got `{spec}`"))?;
    let index = index
        .parse::<u8>()
        .with_context(|| format!("invalid signer index `{index}`"))?;
    ensure!(!value.is_empty(), "empty value for signer index {index}");
    Ok((index, value))
}

/// Parses a secret key given as a BIP-32 xpriv (base58) or 32-byte hex.
pub(crate) fn parse_secret_key(value: &str) -> anyhow::Result<SecretKey> {
    if let Ok(xpriv) = Xpriv::from_str(value) {
        return Ok(xpriv.private_key);
    }
    let bytes = hex::decode(value).context("secret key is neither an xpriv nor hex")?;
    SecretKey::from_slice(&bytes).context("invalid 32-byte secret key")
}

/// Parses `--admin-key <index>:<xpriv|hex>`.
pub(crate) fn parse_admin_key(spec: &str) -> anyhow::Result<(u8, SecretKey)> {
    let (index, value) = split_indexed(spec)?;
    let key = parse_secret_key(value)
        .with_context(|| format!("invalid admin key for signer index {index}"))?;
    Ok((index, key))
}

/// Parses `--signature <index>:<base64 BIP-137 signature>`.
///
/// The 65 raw bytes are kept as emitted (header included); the ASM verifier accepts
/// raw (0-3), compressed P2PKH (31-34), P2SH-P2WPKH (35-38) and P2WPKH (39-42) headers.
pub(crate) fn parse_external_signature(spec: &str) -> anyhow::Result<IndexedSignature> {
    let (index, value) = split_indexed(spec)?;
    let bytes = BASE64
        .decode(value.trim())
        .with_context(|| format!("signature for signer index {index} is not valid base64"))?;
    let sig: [u8; BIP137_SIG_LEN] = bytes.as_slice().try_into().map_err(|_| {
        anyhow!(
            "signature for signer index {index} is {} bytes, expected {BIP137_SIG_LEN}",
            bytes.len()
        )
    })?;
    Ok(IndexedSignature::new(index, sig))
}

/// Signs `digest` with each `(index, key)` and appends `external` signatures.
///
/// Fails on duplicate signer indices or an empty set.
pub(crate) fn assemble_signatures(
    keys: &[(u8, SecretKey)],
    external: Vec<IndexedSignature>,
    digest: &[u8; 32],
) -> anyhow::Result<Vec<IndexedSignature>> {
    let mut sigs: Vec<IndexedSignature> = keys
        .iter()
        .map(|(index, key)| IndexedSignature::new(*index, sign_ecdsa_bip137(digest, key)))
        .collect();
    sigs.extend(external);
    ensure!(!sigs.is_empty(), "no admin keys or signatures supplied");

    let mut indices: Vec<u8> = sigs.iter().map(IndexedSignature::index).collect();
    indices.sort_unstable();
    if let Some(dup) = indices.windows(2).find(|w| w[0] == w[1]) {
        bail!("signer index {} supplied more than once", dup[0]);
    }
    Ok(sigs)
}

/// Loads the threshold config for `role` from an ASM params JSON file.
///
/// Uses ASM's own deserializer (which runs the params invariants) and refuses a file
/// whose magic or network disagrees with what the transaction will be built for.
pub(crate) fn threshold_config_from_asm_params(
    path: &Path,
    role: Role,
    network: Network,
    magic: MagicBytes,
) -> anyhow::Result<ThresholdConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read asm params `{}`", path.display()))?;
    threshold_config_from_asm_params_json(&raw, role, network, magic)
        .with_context(|| format!("asm params `{}`", path.display()))
}

pub(crate) fn threshold_config_from_asm_params_json(
    json: &str,
    role: Role,
    network: Network,
    magic: MagicBytes,
) -> anyhow::Result<ThresholdConfig> {
    let params: AsmParams = serde_json::from_str(json).context("failed to parse asm params")?;
    ensure!(
        params.magic == magic,
        "asm params magic `{}` does not match --magic `{magic}`",
        params.magic
    );
    ensure!(
        params.anchor.network == network,
        "asm params network `{}` does not match --network `{network}`",
        params.anchor.network
    );
    let admin = params
        .admin_config()
        .context("asm params have no Admin subprotocol")?;
    admin
        .check_signer_networks(network)
        .context("asm params signer address network mismatch")?;
    let (_, unchecked) = admin
        .signer_configs()
        .into_iter()
        .find(|(r, _)| *r == role)
        .ok_or_else(|| anyhow!("asm params have no signer config for role {role}"))?;
    ThresholdConfig::try_from(unchecked).with_context(|| format!("invalid config for {role}"))
}

/// Builds a threshold config from explicit P2WPKH signer addresses (in signer-index order).
pub(crate) fn threshold_config_from_addresses(
    addresses: &[String],
    threshold: u8,
    network: Network,
) -> anyhow::Result<ThresholdConfig> {
    let threshold = NonZero::new(threshold).context("threshold must be >= 1")?;
    let signers = addresses
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let addr = Address::<NetworkUnchecked>::from_str(a)
                .with_context(|| format!("invalid signer address #{i} `{a}`"))?;
            ensure!(
                addr.is_valid_for_network(network),
                "signer address #{i} `{a}` is not a {network} address"
            );
            Ok(addr)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let unchecked =
        UncheckedThresholdConfig::try_new(signers, threshold).context("invalid signer set")?;
    ThresholdConfig::try_from(&unchecked).context("invalid signer set")
}

/// Runs the exact on-chain threshold check, first per signature so a failure names the
/// offending signer index and its expected address.
pub(crate) fn verify_signatures(
    config: &ThresholdConfig,
    sigs: &[IndexedSignature],
    digest: &[u8; 32],
    network: Network,
) -> anyhow::Result<()> {
    let per_sig = ThresholdConfig::try_new(config.signers().to_vec(), NonZero::<u8>::MIN)
        .context("failed to build per-signature config")?;
    for sig in sigs {
        let index = sig.index();
        let expected = config
            .signers()
            .get(index as usize)
            .map(|s| s.to_address(network).to_string())
            .unwrap_or_else(|| "<out of bounds>".to_string());
        verify_threshold_signatures(&per_sig, slice::from_ref(sig), digest).map_err(|e| {
            anyhow!("signature for signer index {index} (expected signer {expected}) failed: {e}")
        })?;
    }
    verify_threshold_signatures(config, sigs, digest)
        .map_err(|e| anyhow!("threshold verification failed: {e}"))
}

/// Decodes `reveal_tx` exactly as the ASM does (SPS-50 tag + envelope + SSZ) and checks
/// that it carries `expected`.
pub(crate) fn decode_reveal_tx(
    reveal_tx: &Transaction,
    magic: MagicBytes,
    expected: &SignedPayload,
) -> anyhow::Result<SignedPayload> {
    let tag = ParseConfig::new(magic)
        .try_parse_tx(reveal_tx)
        .map_err(|e| anyhow!("reveal tx has no valid SPS-50 tag: {e}"))?;
    let expected_tag = expected.action.tag();
    ensure!(
        tag.subproto_id() == expected_tag.subproto_id() && tag.tx_type() == expected_tag.tx_type(),
        "reveal tx tag does not match the admin action"
    );
    let decoded = parse_tx(&TxInputRef::new(reveal_tx, tag))
        .context("ASM admin parser rejected the reveal tx")?;
    ensure!(
        &decoded == expected,
        "decoded reveal payload differs from the signed payload"
    );
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        hashes::Hash as _,
        secp256k1::{Message, PublicKey, SECP256K1},
        sign_message::{signed_msg_hash, MessageSignature},
    };
    use strata_asm_admin_threshold_sig::P2wpkhAddress;
    use strata_asm_proto_admin_txs::actions::{
        updates::{EeStfVkUpdate, OlStfVkUpdate},
        UpdateAction,
    };
    use strata_predicate::PredicateKey;

    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/staging_v2_asm_params.json");

    fn keys() -> [SecretKey; 3] {
        [1u8, 2, 3].map(|b| SecretKey::from_slice(&[b; 32]).expect("valid key"))
    }

    fn config_2_of_3(keys: &[SecretKey]) -> ThresholdConfig {
        let signers = keys
            .iter()
            .map(|k| P2wpkhAddress::from_pubkey(&PublicKey::from_secret_key(SECP256K1, k)))
            .collect();
        ThresholdConfig::try_new(signers, NonZero::new(2).expect("non-zero")).expect("config")
    }

    fn ee_action() -> MultisigAction {
        MultisigAction::Update(UpdateAction::EeStfVk(EeStfVkUpdate::new(
            PredicateKey::always_accept(),
        )))
    }

    #[test]
    fn two_of_three_verifies_on_signet_and_bitcoin() {
        let k = keys();
        let config = config_2_of_3(&k);
        for network in [Network::Signet, Network::Bitcoin] {
            let (_, digest) = signing_message(&ee_action(), 7, network);
            let sigs = assemble_signatures(&[(0, k[0]), (2, k[2])], vec![], &digest).unwrap();
            verify_signatures(&config, &sigs, &digest, network).unwrap();
            // Same check the chain runs.
            verify_threshold_signatures(&config, &sigs, &digest).unwrap();
        }
    }

    #[test]
    fn regtest_signed_payload_fails_on_signet() {
        let k = keys();
        let config = config_2_of_3(&k);
        let (_, regtest_digest) = signing_message(&ee_action(), 1, Network::Regtest);
        let (_, signet_digest) = signing_message(&ee_action(), 1, Network::Signet);
        assert_ne!(regtest_digest, signet_digest);
        let sigs = assemble_signatures(&[(0, k[0]), (1, k[1])], vec![], &regtest_digest).unwrap();
        let err = verify_signatures(&config, &sigs, &signet_digest, Network::Signet).unwrap_err();
        assert!(err.to_string().contains("signer index 0"), "{err}");
        assert!(verify_threshold_signatures(&config, &sigs, &signet_digest).is_err());
    }

    #[test]
    fn one_of_three_fails_threshold_two() {
        let k = keys();
        let config = config_2_of_3(&k);
        let (_, digest) = signing_message(&ee_action(), 1, Network::Signet);
        let sigs = assemble_signatures(&[(1, k[1])], vec![], &digest).unwrap();
        let err = verify_signatures(&config, &sigs, &digest, Network::Signet).unwrap_err();
        assert!(err.to_string().contains("insufficient signatures"), "{err}");
    }

    #[test]
    fn wrong_index_is_reported() {
        let k = keys();
        let config = config_2_of_3(&k);
        let (_, digest) = signing_message(&ee_action(), 1, Network::Signet);
        // key 0 claimed as signer 1
        let sigs = assemble_signatures(&[(1, k[0]), (2, k[2])], vec![], &digest).unwrap();
        let err = verify_signatures(&config, &sigs, &digest, Network::Signet).unwrap_err();
        assert!(err.to_string().contains("signer index 1"), "{err}");
    }

    #[test]
    fn duplicate_index_rejected() {
        let k = keys();
        let (_, digest) = signing_message(&ee_action(), 1, Network::Signet);
        assert!(assemble_signatures(&[(0, k[0]), (0, k[1])], vec![], &digest).is_err());
    }

    /// Signatures produced the way a wallet's `signmessage` does (over the message
    /// text, base64-encoded) are accepted, with Electrum-style and BIP-137 P2WPKH headers.
    #[test]
    fn external_base64_signatures_accepted() {
        let k = keys();
        let config = config_2_of_3(&k);
        let (message, digest) = signing_message(&ee_action(), 3, Network::Signet);

        let wallet_sign = |key: &SecretKey, header_offset: u8| {
            let hash = signed_msg_hash(message.as_str());
            let msg = Message::from_digest(hash.to_byte_array());
            let mut bytes =
                MessageSignature::new(SECP256K1.sign_ecdsa_recoverable(&msg, key), true)
                    .serialize();
            bytes[0] += header_offset;
            BASE64.encode(bytes)
        };
        // header 31-34 (Electrum/Sparrow), 39-42 (BIP-137 native segwit, Trezor)
        let ext = vec![
            parse_external_signature(&format!("0:{}", wallet_sign(&k[0], 0))).unwrap(),
            parse_external_signature(&format!("2:{}", wallet_sign(&k[2], 8))).unwrap(),
        ];
        let sigs = assemble_signatures(&[], ext, &digest).unwrap();
        verify_signatures(&config, &sigs, &digest, Network::Signet).unwrap();

        // mixed: one local key + one external
        let ext = vec![parse_external_signature(&format!("1:{}", wallet_sign(&k[1], 0))).unwrap()];
        let sigs = assemble_signatures(&[(0, k[0])], ext, &digest).unwrap();
        verify_signatures(&config, &sigs, &digest, Network::Signet).unwrap();
    }

    #[test]
    fn external_signature_parse_errors() {
        assert!(parse_external_signature("nocolon").is_err());
        assert!(parse_external_signature("x:AAAA").is_err());
        assert!(parse_external_signature("0:!!!").is_err());
        assert!(parse_external_signature("0:AAAA").is_err()); // 3 bytes
    }

    #[test]
    fn admin_key_parses_hex_and_xpriv() {
        let (i, key) = parse_admin_key(&format!("2:{}", hex::encode([7u8; 32]))).unwrap();
        assert_eq!((i, key.secret_bytes()), (2, [7u8; 32]));
        let xpriv = "tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7";
        let (i, key) = parse_admin_key(&format!("0:{xpriv}")).unwrap();
        assert_eq!(i, 0);
        assert_eq!(key, Xpriv::from_str(xpriv).unwrap().private_key);
        assert!(parse_admin_key("0:zz").is_err());
    }

    #[test]
    fn staging_v2_asm_params_fixture_parses() {
        let magic = MagicBytes::new(*b"ALPN");
        let expected = [
            "tb1qfelfap4lxscwyjyl0w4yx7npxvgnfrylg7xfjd",
            "tb1qh60s4paa9jr8cmtcrr2v267hzaq86pfznfc6wu",
            "tb1qwrpseyvhue8zfpmv5m5u95tm4l4yhwalsv705l",
        ];
        for role in [Role::AlpenAdministrator, Role::StrataAdministrator] {
            let config =
                threshold_config_from_asm_params_json(FIXTURE, role, Network::Signet, magic)
                    .unwrap();
            assert_eq!(config.threshold(), 2);
            let addrs: Vec<String> = config
                .signers()
                .iter()
                .map(|s| s.to_address(Network::Signet).to_string())
                .collect();
            assert_eq!(addrs, expected);

            // explicit flags produce the identical config
            let explicit =
                threshold_config_from_addresses(&expected.map(String::from), 2, Network::Signet)
                    .unwrap();
            assert_eq!(explicit, config);
        }
        // wrong network / magic are refused
        assert!(threshold_config_from_asm_params_json(
            FIXTURE,
            Role::AlpenAdministrator,
            Network::Regtest,
            magic
        )
        .is_err());
        assert!(threshold_config_from_asm_params_json(
            FIXTURE,
            Role::AlpenAdministrator,
            Network::Signet,
            MagicBytes::new(*b"XXXX")
        )
        .is_err());
        // signet addresses refused for a bitcoin-network explicit config
        assert!(
            threshold_config_from_addresses(&expected.map(String::from), 2, Network::Bitcoin)
                .is_err()
        );
    }

    #[test]
    fn role_mapping() {
        assert_eq!(ee_action().required_role(), Role::AlpenAdministrator);
        let ol = MultisigAction::Update(UpdateAction::OlStfVk(OlStfVkUpdate::new(
            PredicateKey::always_accept(),
        )));
        assert_eq!(ol.required_role(), Role::StrataAdministrator);
    }
}

//! `gen-asm-params` subcommand: generates ASM params from inputs.

use std::{
    fs,
    num::NonZero,
    path::{Path, PathBuf},
    str::FromStr,
};

use bitcoin::{secp256k1::PublicKey, Network, XOnlyPublicKey};
use serde::Serialize;
use strata_asm_params::{
    AdministrationInitConfig, AsmParams, BridgeV1InitConfig, CheckpointInitConfig,
    ConfirmationDepths, SubprotocolInstance,
};
use strata_asm_proto_bridge_v1_types::SafeHarbourAddress;
use strata_btc_types::BitcoinAmount;
use strata_crypto::{
    aggregate_schnorr_keys, keys::compressed::CompressedPublicKey,
    threshold_signature::ThresholdConfig, EvenPublicKey,
};
use strata_identifiers::Buf32;
use strata_l1_txfmt::MagicBytes;
use strata_ol_genesis::build_genesis_artifacts;
use strata_ol_params::{BridgeParams, OLParams};
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_primitives::bitcoin_bosd::{Descriptor, DescriptorType};

use crate::{
    args::{CmdContext, SubcAsmParams},
    checkpoint_predicate::resolve_checkpoint_predicate,
    util::parse_abbr_amt,
};

/// The default assignment duration in blocks.
const DEFAULT_ASSIGNMENT_DURATION: u16 = 64;

/// The default recovery delay in blocks.
const DEFAULT_RECOVERY_DELAY: u16 = 1_008;

/// The default operator fee in sats (0.5 BTC).
const DEFAULT_OPERATOR_FEE: u64 = 50_000_000;

/// The default confirmation depth for admin subprotocol.
const DEFAULT_CONFIRMATION_DEPTH: u16 = 144;

/// The default allowed seqno gap for admin subprotocol.
const DEFAULT_MAX_SEQNO_GAP: NonZero<u8> = NonZero::new(10).expect("10 is non-zero");

/// Executes the `gen-asm-params` subcommand.
///
/// Generates the ASM params for a Strata network.
/// Either writes to a file or prints to stdout depending on the provided options.
pub(super) fn exec(cmd: SubcAsmParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    // Checked before anything is written: otherwise the `--output` write below would
    // already have clobbered the CLI config by the time we bail.
    if let (Some(out_path), Some(cli_config_path)) = (&cmd.output, &cmd.cli_config) {
        anyhow::ensure!(
            !targets_same_file(out_path, cli_config_path),
            "--cli-config must not point at the same file as --output"
        );
    }

    let magic: MagicBytes = if let Some(name_str) = &cmd.name {
        name_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid magic bytes: {}", e))?
    } else {
        "ALPN".parse().expect("default magic bytes should be valid")
    };

    // Get the genesis L1 anchor.
    let anchor = super::genesis_info::retrieve_l1_anchor(
        cmd.l1_anchor_file.as_deref(),
        cmd.genesis_l1_height,
        ctx,
    )?;

    // Parse operator public keys.
    let mut pubkeys: Vec<PublicKey> = Vec::new();

    if let Some(pks_path) = cmd.op_pks {
        let pks_str = fs::read_to_string(pks_path)?;

        for line in pks_str.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }

            pubkeys.push(PublicKey::from_str(line.trim())?);
        }
    }

    for key in &cmd.op_pk {
        pubkeys.push(PublicKey::from_str(key.trim())?);
    }

    // Build admin subprotocol params using the operator keys for all three admin roles.
    let admin_keys: Vec<CompressedPublicKey> = pubkeys
        .iter()
        .copied()
        .map(CompressedPublicKey::from)
        .collect();

    let threshold = ThresholdConfig::try_new(admin_keys, NonZero::new(1).expect("1 is non-zero"))?;

    let depth = cmd.confirmation_depth.unwrap_or(DEFAULT_CONFIRMATION_DEPTH);
    let confirmation_depths = ConfirmationDepths {
        strata_admin_multisig_update: depth,
        strata_seq_manager_multisig_update: depth,
        alpen_admin_multisig_update: depth,
        strata_security_council_multisig_update: depth,
        operator_update: depth,
        sequencer_update: depth,
        ol_stf_vk_update: depth,
        asm_stf_vk_update: depth,
        ee_stf_vk_update: depth,
        defcon3: depth,
        safe_harbour_address_update: depth,
    };

    let admin = AdministrationInitConfig::new(
        threshold.clone(),
        threshold.clone(),
        threshold.clone(),
        threshold,
        confirmation_depths,
        cmd.max_seqno_gap.unwrap_or(DEFAULT_MAX_SEQNO_GAP),
    );

    // Compute genesis OL block ID from OL params.
    let ol_params_str = fs::read_to_string(&cmd.ol_params)
        .map_err(|e| anyhow::anyhow!("failed to read OL params file {:?}: {e}", cmd.ol_params))?;
    let ol_params: OLParams = serde_json::from_str(&ol_params_str)
        .map_err(|e| anyhow::anyhow!("failed to parse OL params: {e}"))?;

    if ol_params.last_l1_block != anchor.block {
        anyhow::bail!(
            "OL params and ASM params have different genesis L1 block: OL={:?}, ASM={:?}",
            ol_params.last_l1_block,
            anchor.block
        );
    }
    let genesis_artifacts = build_genesis_artifacts(&ol_params)
        .map_err(|e| anyhow::anyhow!("failed to build genesis artifacts: {e}"))?;
    let genesis_ol_blkid = *genesis_artifacts.commitment.blkid();

    // Build checkpoint config.
    let sequencer_predicate = resolve_sequencer_predicate(cmd.seq_pk.as_deref())?;
    let checkpoint_predicate = resolve_checkpoint_predicate(cmd.checkpoint_predicate)?;
    let genesis_l1_height = anchor.block.height();

    let checkpoint = CheckpointInitConfig {
        sequencer_predicate,
        checkpoint_predicate,
        genesis_l1_height,
        genesis_ol_blkid,
    };

    // Build bridge config.
    let deposit_sats =
        resolve_deposit_sats(cmd.deposit_sats.as_deref(), ol_params.bridge_params())?;

    let safe_harbour_address = resolve_safe_harbour_address(&cmd.safe_harbour_address)?;
    let operators: Vec<EvenPublicKey> = pubkeys.into_iter().map(EvenPublicKey::from).collect();

    let bridge = BridgeV1InitConfig {
        operators,
        denomination: BitcoinAmount::from_sat(deposit_sats),
        assignment_duration: cmd
            .assignment_duration
            .unwrap_or(DEFAULT_ASSIGNMENT_DURATION),
        operator_fee: BitcoinAmount::from_sat(cmd.operator_fee.unwrap_or(DEFAULT_OPERATOR_FEE)),
        recovery_delay: cmd.recovery_delay.unwrap_or(DEFAULT_RECOVERY_DELAY),
        safe_harbour_address,
    };

    // Assemble ASM params.
    let asm_params = AsmParams {
        magic,
        anchor,
        subprotocols: vec![
            SubprotocolInstance::Admin(admin),
            SubprotocolInstance::Checkpoint(checkpoint),
            SubprotocolInstance::Bridge(bridge),
        ],
    };

    let params_buf = serde_json::to_string_pretty(&asm_params)?;

    // Built before any write so that a profile we cannot emit fails the run without
    // leaving a half-generated set of params behind.
    let cli_profile = cmd
        .cli_config
        .as_deref()
        .map(|path| {
            build_cli_network_profile(path, &asm_params, ol_params.bridge_params())
                .map(|profile| (path, profile))
        })
        .transpose()?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    if let Some((cli_config_path, profile)) = &cli_profile {
        write_cli_network_profile(cli_config_path, profile)?;
        eprintln!("wrote alpen-cli network profile to {cli_config_path:?}");
    }

    Ok(())
}

/// Reports whether two paths would be written to the same file.
///
/// A lexical comparison is not enough: `./config.toml` and `config.toml` name the
/// same file, as do two symlinks to one target, and the first write would clobber
/// the second path before its own overwrite guard ever ran.
fn targets_same_file(first: &Path, second: &Path) -> bool {
    resolve_write_target(first) == resolve_write_target(second)
}

/// Resolves a path to the file it would be written to.
///
/// Canonicalizes the path when it already exists, which also resolves symlinks.
/// Output paths usually don't exist yet, so fall back to canonicalizing the parent
/// directory and rejoining the file name.
fn resolve_write_target(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        // A bare file name is relative to the working directory.
        _ => Path::new("."),
    };

    match (fs::canonicalize(parent), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => path.to_owned(),
    }
}

/// Resolves the ASM bridge deposit denomination against the OL bridge params.
///
/// The ASM locks deposits at its own denomination while the OL STF validates
/// withdrawals against `bridge_params.denomination`, so the two must agree or
/// funds deposited on L1 cannot be withdrawn. The OL value is the source of
/// truth; `--deposit-sats` may only restate it.
fn resolve_deposit_sats(
    deposit_sats: Option<&str>,
    ol_bridge_params: &BridgeParams,
) -> anyhow::Result<u64> {
    let ol_denomination = ol_bridge_params.denomination();

    let Some(deposit_sats) = deposit_sats else {
        return Ok(ol_denomination);
    };

    let deposit_sats = parse_abbr_amt(deposit_sats)?;
    anyhow::ensure!(
        deposit_sats == ol_denomination,
        "--deposit-sats ({deposit_sats}) must equal the OL params bridge denomination \
         ({ol_denomination}); the ASM deposit denomination and the OL withdrawal denomination are \
         the same network value"
    );

    Ok(deposit_sats)
}

/// Header prepended to the emitted alpen-cli network profile.
const CLI_PROFILE_HEADER: &str = "# Alpen CLI network profile derived from the ASM params.\n\
                                  # Merge these fields into the CLI's config.toml.\n";

/// Network profile fields the alpen wallet CLI reads from its config.toml.
///
/// Must stay in sync with `SettingsFromFile` in `bin/alpen-cli`.
#[derive(Serialize)]
struct CliNetworkProfile {
    network: Network,
    magic_bytes: MagicBytes,
    bridge_pubkey: String,
    bridge_denomination_sats: u64,
    recovery_delay: u16,
    /// Withdrawals are batched in multiples of the denomination, so the CLI needs
    /// the OL's cap to reject amounts the OL STF would reject.
    max_withdrawal_amount_sats: u64,
    max_withdrawal_descriptor_len: u32,
}

/// Resolves the withdrawal cap the CLI profile must carry.
///
/// The CLI reads an absent `max_withdrawal_amount_sats` as the default 10 BTC cap,
/// and TOML has no null, so an uncapped OL cannot be expressed in the profile:
/// emitting one would silently cap a wallet that the OL would let withdraw more.
/// Refuse instead of guessing.
// TODO: teach the CLI config an explicit uncapped spelling and emit that here.
fn cli_withdrawal_cap(ol_bridge_params: &BridgeParams) -> anyhow::Result<u64> {
    ol_bridge_params.max_withdrawal_amount().ok_or_else(|| {
        anyhow::anyhow!(
            "OL params leave the withdrawal amount uncapped, which the alpen-cli config cannot \
             express; set `bridge_params.max_withdrawal_amount` in the OL params or drop \
             --cli-config"
        )
    })
}

/// Derives the aggregated MuSig2 bridge key the CLI locks deposits to.
///
/// Mirrors the ASM bridge subprotocol's operator table: keys are aggregated in
/// registration order with duplicates skipped.
///
/// TODO(STR-3972): replace this hand-copy of `OperatorTable::calculate_aggregated_key`
/// with the real thing once the ASM crate re-exports `OperatorTable` (its
/// `state::operator` module is currently `pub(crate)`); a semantic change upstream
/// (ordering, dedup) would silently diverge from this mirror and lock deposits to the
/// wrong key.
fn derive_bridge_pubkey(operators: &[EvenPublicKey]) -> anyhow::Result<String> {
    let mut keys: Vec<Buf32> = Vec::with_capacity(operators.len());
    for operator in operators {
        let key = Buf32::from(operator.x_only_public_key().0.serialize());
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    anyhow::ensure!(!keys.is_empty(), "bridge requires at least one operator");

    let agg_key = aggregate_schnorr_keys(keys.iter())
        .map_err(|e| anyhow::anyhow!("failed to aggregate operator keys: {e}"))?;

    Ok(hex::encode(agg_key.serialize()))
}

/// Builds the alpen-cli config fields derived from the ASM and OL params.
///
/// Deposit fields come from the ASM bridge subprotocol; the withdrawal fields come
/// from the OL bridge params, which are what the OL STF validates withdrawals against.
///
/// Refuses to overwrite an existing file: the snippet is meant to be merged
/// into the CLI's config.toml, and pointing this at a live config would wipe
/// every other setting in it.
fn build_cli_network_profile(
    path: &Path,
    asm_params: &AsmParams,
    ol_bridge_params: &BridgeParams,
) -> anyhow::Result<CliNetworkProfile> {
    anyhow::ensure!(
        !path.exists(),
        "refusing to overwrite existing file {path:?}; merge the generated snippet into the CLI \
         config manually"
    );

    let bridge = asm_params
        .bridge_config()
        .ok_or_else(|| anyhow::anyhow!("ASM params missing Bridge subprotocol config"))?;

    Ok(CliNetworkProfile {
        network: asm_params.anchor.network,
        magic_bytes: asm_params.magic,
        bridge_pubkey: derive_bridge_pubkey(&bridge.operators)?,
        bridge_denomination_sats: ol_bridge_params.denomination(),
        recovery_delay: bridge.recovery_delay,
        max_withdrawal_amount_sats: cli_withdrawal_cap(ol_bridge_params)?,
        max_withdrawal_descriptor_len: ol_bridge_params.max_withdrawal_descriptor_len(),
    })
}

/// Writes a network profile as a TOML snippet for the alpen wallet CLI.
fn write_cli_network_profile(path: &Path, profile: &CliNetworkProfile) -> anyhow::Result<()> {
    let body = toml::to_string(profile)?;
    fs::write(path, format!("{CLI_PROFILE_HEADER}{body}"))?;

    Ok(())
}

fn resolve_sequencer_predicate(seq_pk: Option<&str>) -> anyhow::Result<PredicateKey> {
    let Some(pk_hex) = seq_pk.map(str::trim) else {
        return Ok(PredicateKey::always_accept());
    };

    let xonly = XOnlyPublicKey::from_str(pk_hex)?;
    Ok(PredicateKey::new(
        PredicateTypeId::Bip340Schnorr,
        xonly.serialize().to_vec(),
    ))
}

fn resolve_safe_harbour_address(descriptor: &str) -> anyhow::Result<SafeHarbourAddress> {
    let descriptor = descriptor.trim();
    anyhow::ensure!(
        !descriptor.is_empty(),
        "safe harbour descriptor must not be empty"
    );

    let descriptor = descriptor
        .parse::<Descriptor>()
        .map_err(|e| anyhow::anyhow!("invalid safe harbour descriptor: {e}"))?;
    anyhow::ensure!(
        descriptor.type_tag() == DescriptorType::P2tr,
        "safe harbour descriptor must be P2TR, got {}",
        descriptor.type_tag()
    );

    SafeHarbourAddress::try_from(descriptor)
        .map_err(|e| anyhow::anyhow!("invalid safe harbour descriptor: {e}"))
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    use super::*;

    /// X-only pubkey hex derived from the test xpriv
    /// `tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7`
    /// via `genseqpubkey`.
    const TEST_SEQ_PK: &str = "14ebfa9a90fee3020686b5334b297b675a9f29282f44b6c3a4ab1f0582021839";

    #[test]
    fn cli_network_profile_matches_cli_config_schema() {
        let profile = CliNetworkProfile {
            network: Network::Signet,
            magic_bytes: "ALPN".parse().expect("valid magic bytes"),
            bridge_pubkey: TEST_SEQ_PK.to_owned(),
            bridge_denomination_sats: 100_000_000,
            recovery_delay: 1_008,
            max_withdrawal_amount_sats: 1_000_000_000,
            max_withdrawal_descriptor_len: 81,
        };

        let rendered = format!(
            "{CLI_PROFILE_HEADER}{}",
            toml::to_string(&profile).expect("profile should serialize")
        );

        // Must stay byte-identical to the literal parsed by
        // `test_parses_datatool_network_profile_snippet` in bin/alpen-cli, so
        // a field rename on either side fails one of the two tests.
        assert_eq!(
            rendered,
            "# Alpen CLI network profile derived from the ASM params.\n\
             # Merge these fields into the CLI's config.toml.\n\
             network = \"signet\"\n\
             magic_bytes = \"ALPN\"\n\
             bridge_pubkey = \"14ebfa9a90fee3020686b5334b297b675a9f29282f44b6c3a4ab1f0582021839\"\n\
             bridge_denomination_sats = 100000000\n\
             recovery_delay = 1008\n\
             max_withdrawal_amount_sats = 1000000000\n\
             max_withdrawal_descriptor_len = 81\n"
        );
    }

    #[test]
    fn cli_withdrawal_cap_uses_the_ol_cap() {
        let ol_bridge_params =
            BridgeParams::new(100_000_000, Some(500_000_000)).expect("valid bridge params");

        let cap = cli_withdrawal_cap(&ol_bridge_params).expect("capped params resolve");

        assert_eq!(cap, 500_000_000);
    }

    #[test]
    fn cli_withdrawal_cap_rejects_uncapped_ol_params() {
        // The CLI reads an absent cap as the default 10 BTC, so an uncapped OL cannot
        // be expressed: emitting a profile would silently cap the wallet below the OL.
        let ol_bridge_params = BridgeParams::new(100_000_000, None).expect("valid bridge params");

        let err = cli_withdrawal_cap(&ol_bridge_params).unwrap_err();

        assert!(err.to_string().contains("uncapped"));
    }

    #[test]
    fn same_file_targets_are_detected_through_equivalent_spellings() {
        assert!(targets_same_file(
            Path::new("./Cargo.toml"),
            Path::new("Cargo.toml")
        ));
        assert!(targets_same_file(
            Path::new("src/../Cargo.toml"),
            Path::new("Cargo.toml")
        ));
    }

    #[test]
    fn distinct_file_targets_are_not_conflated() {
        assert!(!targets_same_file(
            Path::new("asm-params.json"),
            Path::new("cli-config.toml")
        ));
    }

    #[test]
    fn deposit_sats_defaults_to_the_ol_denomination() {
        let ol_bridge_params = BridgeParams::new(50_000_000, None).expect("valid bridge params");

        let deposit_sats =
            resolve_deposit_sats(None, &ol_bridge_params).expect("default should resolve");

        assert_eq!(deposit_sats, 50_000_000);
    }

    #[test]
    fn deposit_sats_may_restate_the_ol_denomination() {
        let ol_bridge_params = BridgeParams::new(100_000_000, None).expect("valid bridge params");

        let deposit_sats =
            resolve_deposit_sats(Some("100M"), &ol_bridge_params).expect("matching value is fine");

        assert_eq!(deposit_sats, 100_000_000);
    }

    #[test]
    fn deposit_sats_rejects_denomination_mismatch() {
        let ol_bridge_params = BridgeParams::new(100_000_000, None).expect("valid bridge params");

        let err = resolve_deposit_sats(Some("200M"), &ol_bridge_params).unwrap_err();

        assert!(err.to_string().contains("must equal the OL params bridge"));
    }

    #[test]
    fn bridge_pubkey_derivation_skips_duplicates_like_the_operator_table() {
        let secp = Secp256k1::new();
        let operator_1 = EvenPublicKey::from(PublicKey::from_secret_key(
            &secp,
            &SecretKey::from_slice(&[1u8; 32]).expect("valid secret key"),
        ));
        let operator_2 = EvenPublicKey::from(PublicKey::from_secret_key(
            &secp,
            &SecretKey::from_slice(&[2u8; 32]).expect("valid secret key"),
        ));

        let derived =
            derive_bridge_pubkey(&[operator_1, operator_2, operator_1]).expect("keys aggregate");

        let deduped_keys: Vec<Buf32> = [operator_1, operator_2]
            .iter()
            .map(|op| Buf32::from(op.x_only_public_key().0.serialize()))
            .collect();
        let expected = hex::encode(
            aggregate_schnorr_keys(deduped_keys.iter())
                .expect("keys aggregate")
                .serialize(),
        );

        assert_eq!(derived, expected);
    }

    #[test]
    fn bridge_pubkey_derivation_rejects_empty_operator_set() {
        assert!(derive_bridge_pubkey(&[]).is_err());
    }

    #[test]
    fn sequencer_predicate_defaults_to_always_accept() {
        let predicate = resolve_sequencer_predicate(None).expect("default should resolve");

        assert_eq!(predicate.id(), PredicateTypeId::AlwaysAccept.as_u8());
    }

    #[test]
    fn sequencer_predicate_uses_bip340_schnorr_pubkey() {
        let predicate =
            resolve_sequencer_predicate(Some(TEST_SEQ_PK)).expect("x-only hex should parse");
        let xonly = XOnlyPublicKey::from_str(TEST_SEQ_PK).expect("x-only hex should parse");
        let expected_pubkey = xonly.serialize();

        assert_eq!(predicate.id(), PredicateTypeId::Bip340Schnorr.as_u8());
        assert_eq!(predicate.condition(), expected_pubkey.as_slice());
    }

    #[test]
    fn safe_harbour_address_accepts_p2tr_descriptor() {
        let xonly = XOnlyPublicKey::from_str(TEST_SEQ_PK).expect("x-only hex should parse");
        let descriptor =
            Descriptor::new_p2tr(&xonly.serialize()).expect("x-only pubkey should be valid p2tr");

        let resolved = resolve_safe_harbour_address(&descriptor.to_string())
            .expect("p2tr descriptor should resolve");

        assert_eq!(resolved.as_descriptor(), &descriptor);
    }

    #[test]
    fn safe_harbour_address_rejects_non_p2tr_descriptor() {
        let descriptor = Descriptor::new_p2wpkh(&[1; 20]);

        let err = resolve_safe_harbour_address(&descriptor.to_string()).unwrap_err();

        assert!(err.to_string().contains("must be P2TR"));
    }
}

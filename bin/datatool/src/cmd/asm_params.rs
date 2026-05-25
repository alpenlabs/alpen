//! `gen-asm-params` subcommand: generates ASM params from inputs.

use std::{fs, num::NonZero, str::FromStr};

use bitcoin::{XOnlyPublicKey, hashes::{Hash, hash160}, secp256k1::PublicKey};
use strata_asm_params::{
    AdministrationInitConfig, AsmParams, BridgeV1InitConfig, CheckpointInitConfig,
    ConfirmationDepths, SubprotocolInstance,
};
use strata_btc_types::BitcoinAmount;
use strata_btc_verification::L1Anchor;
use strata_crypto::{
    keys::compressed::CompressedPublicKey, threshold_signature::ThresholdConfig, EvenPublicKey,
};
use strata_l1_txfmt::MagicBytes;
use strata_ol_genesis::build_genesis_artifacts;
use strata_ol_params::OLParams;
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{
    args::{CmdContext, SubcAsmParams},
    checkpoint_predicate::resolve_checkpoint_predicate,
    util::parse_abbr_amt,
};

/// The default deposit amount in sats (1 BTC).
const DEFAULT_DEPOSIT_SATS: u64 = 100_000_000;

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
    let magic: MagicBytes = if let Some(name_str) = &cmd.name {
        name_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid magic bytes: {}", e))?
    } else {
        "ALPN".parse().expect("default magic bytes should be valid")
    };

    // Get genesis L1 view.
    let genesis_l1_view = super::params::retrieve_genesis_l1_view(
        cmd.genesis_l1_view_file.as_deref(),
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
        operator_update: depth,
        sequencer_update: depth,
        ol_stf_vk_update: depth,
        asm_stf_vk_update: depth,
        ee_stf_vk_update: depth,
    };

    let admin = AdministrationInitConfig::new(
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

    if ol_params.last_l1_block != genesis_l1_view.blk {
        anyhow::bail!(
            "OL params and ASM params have different genesis L1 block: OL={:?}, ASM={:?}",
            ol_params.last_l1_block,
            genesis_l1_view.blk
        );
    }
    let genesis_artifacts = build_genesis_artifacts(&ol_params)
        .map_err(|e| anyhow::anyhow!("failed to build genesis artifacts: {e}"))?;
    let genesis_ol_blkid = *genesis_artifacts.commitment.blkid();

    // Build checkpoint config.
    let sequencer_predicate = resolve_sequencer_predicate(cmd.seq_pk.as_deref())?;
    let checkpoint_predicate = resolve_checkpoint_predicate(cmd.checkpoint_predicate)?;
    let genesis_l1_height = genesis_l1_view.blk.height();

    let checkpoint = CheckpointInitConfig {
        sequencer_predicate,
        checkpoint_predicate,
        genesis_l1_height,
        genesis_ol_blkid,
    };

    // Build bridge config.
    let deposit_sats = cmd
        .deposit_sats
        .map(|s| parse_abbr_amt(&s))
        .transpose()?
        .unwrap_or(DEFAULT_DEPOSIT_SATS);

    let operators: Vec<EvenPublicKey> = pubkeys.into_iter().map(EvenPublicKey::from).collect();

    let bridge = BridgeV1InitConfig {
        operators,
        denomination: BitcoinAmount::from_sat(deposit_sats),
        assignment_duration: cmd
            .assignment_duration
            .unwrap_or(DEFAULT_ASSIGNMENT_DURATION),
        operator_fee: BitcoinAmount::from_sat(cmd.operator_fee.unwrap_or(DEFAULT_OPERATOR_FEE)),
        recovery_delay: cmd.recovery_delay.unwrap_or(DEFAULT_RECOVERY_DELAY),
    };

    // Assemble ASM params.
    let anchor = L1Anchor {
        block: genesis_l1_view.blk,
        next_target: genesis_l1_view.next_target,
        epoch_start_timestamp: genesis_l1_view.epoch_start_timestamp,
        network: ctx.bitcoin_network,
    };
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

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// X-only pubkey hex derived from the test xpriv
    /// `tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7`
    /// via `genseqpubkey`.
    const TEST_SEQ_PK: &str = "14ebfa9a90fee3020686b5334b297b675a9f29282f44b6c3a4ab1f0582021839";

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
}

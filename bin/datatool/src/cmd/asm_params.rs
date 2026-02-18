//! `gen-asm-params` subcommand: generates ASM params from inputs.

use std::{fs, num::NonZero, str::FromStr};

use bitcoin::{
    bip32::Xpriv,
    secp256k1::{PublicKey, SECP256K1},
};
use strata_asm_params::{
    AdministrationSubprotoParams, AsmParams, BridgeV1Config, CheckpointConfig, SubprotocolInstance,
};
use strata_btc_types::BitcoinAmount;
use strata_crypto::{
    keys::compressed::CompressedPublicKey, threshold_signature::ThresholdConfig, EvenPublicKey,
};
use strata_identifiers::OLBlockId;
use strata_l1_txfmt::MagicBytes;
use strata_predicate::PredicateKey;

use crate::{
    args::{CmdContext, SubcAsmParams},
    checkpoint_predicate::resolve_checkpoint_predicate,
    util::parse_abbr_amt,
};

/// The default deposit amount in sats (10 BTC).
const DEFAULT_DEPOSIT_SATS: u64 = 1_000_000_000;

/// The default assignment duration in blocks.
const DEFAULT_ASSIGNMENT_DURATION: u16 = 64;

/// The default recovery delay in blocks.
const DEFAULT_RECOVERY_DELAY: u16 = 1_008;

/// The default operator fee in sats (0.5 BTC).
const DEFAULT_OPERATOR_FEE: u64 = 50_000_000;

/// The default confirmation depth for admin subprotocol.
const DEFAULT_CONFIRMATION_DEPTH: u16 = 144;

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
        "alpn".parse().expect("default magic bytes should be valid")
    };

    // Get genesis L1 view.
    let genesis_l1_view = super::params::retrieve_genesis_l1_view(
        cmd.genesis_l1_view_file.as_deref(),
        cmd.genesis_l1_height,
        ctx,
    )?;

    // Parse operator keys.
    let mut opkeys = Vec::new();

    if let Some(opkeys_path) = cmd.opkeys {
        let opkeys_str = fs::read_to_string(opkeys_path)?;

        for line in opkeys_str.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }

            opkeys.push(Xpriv::from_str(line)?);
        }
    }

    for key in cmd.opkey {
        opkeys.push(Xpriv::from_str(&key)?);
    }

    // Convert xpriv keys to secp256k1 public keys.
    let pubkeys: Vec<PublicKey> = opkeys
        .iter()
        .map(|xpriv| xpriv.to_keypair(SECP256K1).public_key())
        .collect();

    // Build admin subprotocol params using the first operator's key for both threshold configs.
    let admin_keys: Vec<CompressedPublicKey> = pubkeys
        .iter()
        .copied()
        .map(CompressedPublicKey::from)
        .collect();

    let threshold = ThresholdConfig::try_new(admin_keys, NonZero::new(1).expect("1 is non-zero"))?;

    let admin = AdministrationSubprotoParams::new(
        threshold.clone(),
        threshold,
        cmd.confirmation_depth.unwrap_or(DEFAULT_CONFIRMATION_DEPTH),
    );

    // Build checkpoint config.
    let checkpoint_predicate = resolve_checkpoint_predicate();
    let genesis_l1_height = genesis_l1_view.blk.height_u32();

    let checkpoint = CheckpointConfig {
        sequencer_predicate: PredicateKey::always_accept(),
        checkpoint_predicate,
        genesis_l1_height,
        genesis_ol_blkid: OLBlockId::null(),
    };

    // Build bridge config.
    let deposit_sats = cmd
        .deposit_sats
        .map(|s| parse_abbr_amt(&s))
        .transpose()?
        .unwrap_or(DEFAULT_DEPOSIT_SATS);

    let operators: Vec<EvenPublicKey> = pubkeys.into_iter().map(EvenPublicKey::from).collect();

    let bridge = BridgeV1Config {
        operators,
        denomination: BitcoinAmount::from_sat(deposit_sats),
        assignment_duration: cmd
            .assignment_duration
            .unwrap_or(DEFAULT_ASSIGNMENT_DURATION),
        operator_fee: BitcoinAmount::from_sat(cmd.operator_fee.unwrap_or(DEFAULT_OPERATOR_FEE)),
        recovery_delay: cmd.recovery_delay.unwrap_or(DEFAULT_RECOVERY_DELAY),
    };

    // Assemble ASM params.
    let asm_params = AsmParams {
        magic,
        l1_view: genesis_l1_view,
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

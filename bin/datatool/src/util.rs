//! Utility functions for Strata `datatool` binary.
//!
//! It contains functions for generating keys, parsing amounts, and constructing
//! network parameters.
//! These functions are called from the CLI's subcommands.

use std::{
    fs,
    path::{Path, PathBuf},
};

use bitcoin::{
    base58,
    bip32::{Xpriv, Xpub},
    Network,
};
use rand::{CryptoRng, RngCore};
use strata_key_derivation::{
    operator::{convert_base_xpub_to_message_xpub, convert_base_xpub_to_wallet_xpub, OperatorKeys},
    sequencer::SequencerKeys,
};
use strata_primitives::{
    block_credential,
    buf::Buf32,
    keys::ZeroizableXpriv,
    operator::OperatorPubkeys,
    params::{ProofPublishMode, RollupParams},
    proof::RollupVerifyingKey,
};
use strata_sp1_guest_builder::GUEST_CHECKPOINT_VK_HASH_STR;
use zeroize::Zeroize;

use crate::args::{
    CmdContext, SubcOpXpub, SubcParams, SubcSeqPrivkey, SubcSeqPubkey, SubcXpriv, Subcommand,
};

/// Sequencer key environment variable.
const SEQKEY_ENVVAR: &str = "STRATA_SEQ_KEY";

/// Operator key environment variable.
const OPKEY_ENVVAR: &str = "STRATA_OP_KEY";

/// The default network to use.
///
/// Right now this is [`Network::Signet`].
const DEFAULT_NETWORK: Network = Network::Signet;

/// Resolves a [`Network`] from a string.
pub(super) fn resolve_network(arg: Option<&str>) -> anyhow::Result<Network> {
    match arg {
        Some("signet") => Ok(Network::Signet),
        Some("regtest") => Ok(Network::Regtest),
        Some(n) => anyhow::bail!("unsupported network option: {n}"),
        None => Ok(DEFAULT_NETWORK),
    }
}

/// Executes a `gen*` subcommand.
pub(super) fn exec_subc(cmd: Subcommand, ctx: &mut CmdContext) -> anyhow::Result<()> {
    match cmd {
        Subcommand::Xpriv(subc) => exec_genxpriv(subc, ctx),
        Subcommand::SeqPubkey(subc) => exec_genseqpubkey(subc, ctx),
        Subcommand::SeqPrivkey(subc) => exec_genseqprivkey(subc, ctx),
        Subcommand::OpXpub(subc) => exec_genopxpub(subc, ctx),
        Subcommand::Params(subc) => exec_genparams(subc, ctx),
    }
}

/// Executes the `genxpriv` subcommand.
///
/// Generates a new [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`] and writes it to a file.
fn exec_genxpriv(cmd: SubcXpriv, ctx: &mut CmdContext) -> anyhow::Result<()> {
    if cmd.path.exists() && !cmd.force {
        anyhow::bail!("not overwriting file, add --force to overwrite");
    }

    let xpriv = gen_priv(&mut ctx.rng, ctx.bitcoin_network);
    let mut buf = xpriv.encode();
    let mut s = base58::encode_check(&buf);
    let result = fs::write(&cmd.path, s.as_bytes());

    buf.zeroize();
    s.zeroize();

    match result {
        Ok(_) => Ok(()),
        Err(_) => anyhow::bail!("failed to write to file {:?}", cmd.path),
    }
}

/// Executes the `genseqpubkey` subcommand.
///
/// Generates the sequencer [`Xpub`] from the provided [`Xpriv`]
/// and prints it to stdout.
fn exec_genseqpubkey(cmd: SubcSeqPubkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, SEQKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let seq_keys = SequencerKeys::new(&xpriv)?;
    let seq_xpub = seq_keys.derived_xpub();
    let raw_buf = seq_xpub.to_x_only_pub().serialize();
    let s = base58::encode_check(&raw_buf);

    println!("{s}");

    Ok(())
}

/// Executes the `genseqprivkey` subcommand.
///
/// Generates the sequencer [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`] and prints it to
/// stdout.
fn exec_genseqprivkey(cmd: SubcSeqPrivkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, SEQKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let seq_keys = SequencerKeys::new(&xpriv)?;
    let seq_xpriv = seq_keys.derived_xpriv();
    let mut raw_buf = seq_xpriv.to_priv().to_bytes();
    let mut s = base58::encode_check(&raw_buf);

    println!("{s}");

    // Zeroize the buffers after printing.
    raw_buf.zeroize();
    s.zeroize();

    Ok(())
}

/// Executes the `genopxpub` subcommand.
///
/// Generates the root xpub for an operator.
fn exec_genopxpub(cmd: SubcOpXpub, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, OPKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let op_keys = OperatorKeys::new(&xpriv)?;
    let op_base_xpub = op_keys.base_xpub();
    let raw_buf = op_base_xpub.encode();
    let s = base58::encode_check(&raw_buf);

    println!("{s}");

    Ok(())
}

/// Executes the `genparams` subcommand.
///
/// Generates the params for a Strata network.
/// Either writes to a file or prints to stdout depending on the provided options.
fn exec_genparams(cmd: SubcParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    // Parse the sequencer key, trimming whitespace for convenience.
    let seqkey = match cmd.seqkey.as_ref().map(|s| s.trim()) {
        Some(seqkey) => {
            let buf = match base58::decode_check(seqkey) {
                Ok(v) => v,
                Err(e) => {
                    anyhow::bail!("failed to parse sequencer key '{seqkey}': {e}");
                }
            };

            let Ok(buf) = Buf32::try_from(buf.as_slice()) else {
                anyhow::bail!("invalid sequencer key '{seqkey}' (must be 32 bytes)");
            };

            Some(buf)
        }
        None => None,
    };

    // Parse each of the operator keys.
    let mut opkeys = Vec::new();

    if let Some(opkeys_path) = cmd.opkeys {
        let opkeys_str = fs::read_to_string(opkeys_path)?;

        for l in opkeys_str.lines() {
            // skip lines that are empty or look like comments
            if l.trim().is_empty() || l.starts_with("#") {
                continue;
            }

            opkeys.push(parse_xpub(l)?);
        }
    }

    for k in cmd.opkey {
        opkeys.push(parse_xpub(&k)?);
    }

    // Parse the deposit size str.
    let deposit_sats = cmd
        .deposit_sats
        .map(|s| parse_abbr_amt(&s))
        .transpose()?
        .unwrap_or(1_000_000_000);

    // Parse the checkpoint verification key.
    let rollup_vk: Buf32 = GUEST_CHECKPOINT_VK_HASH_STR
        .parse()
        .expect("invalid checkpoint verifier key hash");

    let config = ParamsConfig {
        name: cmd.name.unwrap_or_else(|| "strata-testnet".to_string()),
        bitcoin_network: ctx.bitcoin_network,
        // TODO make these consts
        block_time_sec: cmd.block_time.unwrap_or(15),
        epoch_slots: cmd.epoch_slots.unwrap_or(64),
        genesis_trigger: cmd.genesis_trigger_height.unwrap_or(100),
        seqkey,
        opkeys,
        rollup_vk,
        // TODO make a const
        deposit_sats,
        proof_timeout: cmd.proof_timeout,
    };

    let params = construct_params(config);
    let params_buf = serde_json::to_string_pretty(&params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}

/// Generates a new [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`].
///
/// # Notes
///
/// Takes a mutable reference to an RNG to allow flexibility in testing.
/// The actual generation requires a high-entropy source like [`OsRng`](rand::rngs::OsRng)
/// to securely generate extended private keys.
fn gen_priv<R: CryptoRng + RngCore>(rng: &mut R, net: Network) -> ZeroizableXpriv {
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    let mut xpriv = Xpriv::new_master(net, &seed).expect("valid seed");
    let zeroizable_xpriv: ZeroizableXpriv = xpriv.into();

    // Zeroize the seed after generating the xpriv.
    seed.zeroize();
    // Zeroize the xpriv after generating it.
    //
    // NOTE: `zeroizable_xpriv` is zeroized on drop.
    xpriv.private_key.non_secure_erase();

    zeroizable_xpriv
}

/// Reads an [`Xpriv`] from file as a string and verifies the checksum.
///
/// # Notes
///
/// This [`Xpriv`] will [`Zeroize`](zeroize) on [`Drop`].
fn read_xpriv(path: &Path) -> anyhow::Result<ZeroizableXpriv> {
    let mut raw_buf = fs::read(path)?;
    let str_buf: &str = std::str::from_utf8(&raw_buf)?;
    let mut buf = base58::decode_check(str_buf)?;

    // Parse into a ZeroizableXpriv.
    let xpriv = Xpriv::decode(&buf)?;
    let zeroizable_xpriv: ZeroizableXpriv = xpriv.into();

    // Zeroize the buffers after parsing.
    //
    // NOTE: `zeroizable_xpriv` is zeroized on drop;
    //        and `str_buf` is a reference to `raw_buf`.
    raw_buf.zeroize();
    buf.zeroize();

    Ok(zeroizable_xpriv)
}

/// Parses an [`Xpriv`] from environment variable.
///
/// # Notes
///
/// This [`Xpriv`] will [`Zeroize`](zeroize) on [`Drop`].
fn parse_xpriv_from_env(env: &'static str) -> anyhow::Result<Option<ZeroizableXpriv>> {
    let mut env_val = match std::env::var(env) {
        Ok(v) => v,
        Err(_) => anyhow::bail!("got --key-from-env but {env} not set or invalid"),
    };

    let mut buf = base58::decode_check(&env_val)?;

    // Parse into a ZeroizableXpriv.
    let mut xpriv = Xpriv::decode(&buf)?;
    let zeroizable_xpriv: ZeroizableXpriv = xpriv.into();

    // Zeroize the buffers after parsing.
    //
    // NOTE: `zeroizable_xpriv` is zeroized on drop.
    env_val.zeroize();
    buf.zeroize();
    xpriv.private_key.non_secure_erase();

    Ok(Some(zeroizable_xpriv))
}

/// Parses an [`Xpriv`] from file path.
///
/// # Notes
///
/// This [`Xpriv`] will [`Zeroize`](zeroize) on [`Drop`].
fn parse_xpriv_from_path(path: &Path) -> anyhow::Result<Option<ZeroizableXpriv>> {
    Ok(Some(read_xpriv(path)?))
}

/// Resolves an [`Xpriv`] from the file path (if provided) or environment variable (if
/// `--key-from-env` set). Only one source should be specified.
///
/// Priority:
///
/// 1. File path (if provided with path argument)
/// 2. Environment variable (if --key-from-env flag is set)
///
/// # Notes
///
/// This [`Xpriv`] will [`Zeroize`](zeroize) on [`Drop`].
fn resolve_xpriv(
    path: &Option<PathBuf>,
    from_env: bool,
    env: &'static str,
) -> anyhow::Result<Option<ZeroizableXpriv>> {
    match (path, from_env) {
        (Some(_), true) => anyhow::bail!("got key path and --key-from-env, pick a lane"),
        (Some(path), false) => parse_xpriv_from_path(path),
        (None, true) => parse_xpriv_from_env(env),
        _ => Ok(None),
    }
}

/// Inputs for constructing the network parameters.
pub struct ParamsConfig {
    /// Name of the network.
    name: String,
    /// Network to use.
    #[allow(unused)]
    bitcoin_network: Network,
    /// Block time in seconds.
    block_time_sec: u64,
    /// Number of slots in an epoch.
    epoch_slots: u32,
    /// Height at which the genesis block is triggered.
    genesis_trigger: u64,
    /// Sequencer's key.
    seqkey: Option<Buf32>,
    /// Operators' keys.
    opkeys: Vec<Xpub>,
    /// Verifier's key.
    rollup_vk: Buf32,
    /// Amount of sats to deposit.
    deposit_sats: u64,
    /// Timeout for proofs.
    proof_timeout: Option<u32>,
}

/// Constructs the parameters for a Strata network.
// TODO convert this to also initialize the sync params
fn construct_params(config: ParamsConfig) -> RollupParams {
    let cr = config
        .seqkey
        .map(block_credential::CredRule::SchnorrKey)
        .unwrap_or(block_credential::CredRule::Unchecked);

    let opkeys = config
        .opkeys
        .into_iter()
        .map(|xpk| {
            let message_xpub = convert_base_xpub_to_message_xpub(&xpk);
            let wallet_xpub = convert_base_xpub_to_wallet_xpub(&xpk);
            let message_key_buf = message_xpub.to_x_only_pub().serialize().into();
            let wallet_key_buf = wallet_xpub.to_x_only_pub().serialize().into();
            OperatorPubkeys::new(message_key_buf, wallet_key_buf)
        })
        .collect::<Vec<_>>();

    // TODO add in bitcoin network
    RollupParams {
        rollup_name: config.name,
        block_time: config.block_time_sec * 1000,
        cred_rule: cr,
        // TODO do we want to remove this?
        horizon_l1_height: config.genesis_trigger / 2,
        genesis_l1_height: config.genesis_trigger,
        operator_config: strata_primitives::params::OperatorConfig::Static(opkeys),
        // TODO make configurable
        evm_genesis_block_hash:
            "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
                .parse()
                .unwrap(),
        evm_genesis_block_state_root:
            "0x351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f"
                .parse()
                .unwrap(),
        // TODO make configurable
        l1_reorg_safe_depth: 4,
        target_l2_batch_size: config.epoch_slots as u64,
        address_length: 20,
        deposit_amount: config.deposit_sats,
        rollup_vk: RollupVerifyingKey::SP1VerifyingKey(config.rollup_vk),
        // TODO make configurable
        dispatch_assignment_dur: 64,
        proof_publish_mode: config
            .proof_timeout
            .map(|t| ProofPublishMode::Timeout(t as u64))
            .unwrap_or(ProofPublishMode::Strict),
        // TODO make configurable
        max_deposits_in_block: 16,
        network: config.bitcoin_network,
    }
}

/// Parses an [`Xpub`] from [`&str`], richly generating [`anyhow::Result`]s from
/// it.
fn parse_xpub(s: &str) -> anyhow::Result<Xpub> {
    let Ok(buf) = base58::decode_check(s) else {
        anyhow::bail!("failed to parse key: {s}");
    };

    let Ok(xpk) = Xpub::decode(&buf) else {
        anyhow::bail!("failed to decode key: {s}");
    };

    Ok(xpk)
}

/// Parses an abbreviated amount string.
///
/// User may of may not use suffixes to denote the amount.
///
/// # Possible suffixes (case sensitive)
///
/// - `K` for thousand.
/// - `M` for million.
/// - `G` for billion.
/// - `T` for trillion.
fn parse_abbr_amt(s: &str) -> anyhow::Result<u64> {
    // Thousand.
    if let Some(v) = s.strip_suffix("K") {
        return Ok(v.parse::<u64>()? * 1000);
    }

    // Million.
    if let Some(v) = s.strip_suffix("M") {
        return Ok(v.parse::<u64>()? * 1_000_000);
    }

    // Billion.
    if let Some(v) = s.strip_suffix("G") {
        return Ok(v.parse::<u64>()? * 1_000_000_000);
    }

    // Trillion, probably not necessary.
    if let Some(v) = s.strip_suffix("T") {
        return Ok(v.parse::<u64>()? * 1_000_000_000_000);
    }

    // Simple value.
    Ok(s.parse::<u64>()?)
}

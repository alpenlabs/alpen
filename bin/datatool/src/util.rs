//! Utility functions for Strata `datatool` binary.
//!
//! It contains functions for generating keys, parsing amounts, and constructing
//! network parameters.
//! These functions are called from the CLI's subcommands.

use std::{
    env, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use alloy_genesis::Genesis;
use alloy_primitives::B256;
use bitcoin::{
    bip32::{Xpriv, Xpub},
    secp256k1::SECP256K1,
    Amount, Network, XOnlyPublicKey,
};
use rand_core::CryptoRngCore;
use reth_chainspec::ChainSpec;
use strata_crypto::keys::zeroizable::ZeroizableXpriv;
use strata_key_derivation::{error::KeyError, sequencer::SequencerKeys};
use strata_l1_txfmt::MagicBytes;
use strata_params::{ProofPublishMode, RollupParams};
use strata_predicate::PredicateKey;
use strata_primitives::{block_credential, buf::Buf32, l1::GenesisL1View};
use zeroize::Zeroize;

use crate::args::{
    CmdContext, SubcGenL1View, SubcParams, SubcSeqPrivkey, SubcSeqPubkey, SubcXpriv, Subcommand,
};

/// Sequencer key environment variable.
const SEQKEY_ENVVAR: &str = "STRATA_SEQ_KEY";

/// Bitcoin network environment variable.
const BITCOIN_NETWORK_ENVVAR: &str = "BITCOIN_NETWORK";

/// The default network to use.
///
/// Right now this is [`Network::Signet`].
const DEFAULT_NETWORK: Network = Network::Signet;

/// The default L1 genesis height to use.
const DEFAULT_L1_GENESIS_HEIGHT: u64 = 100;

/// The default evm chainspec to use in params.
const DEFAULT_CHAIN_SPEC: &str = alpen_chainspec::DEV_CHAIN_SPEC;

/// The default recovery delay to use in params.
const DEFAULT_RECOVERY_DELAY: u32 = 1_008;

/// Resolves a [`Network`] from a string.
///
/// Priority:
///
/// 1. Command-line argument (if provided)
/// 2. `BITCOIN_NETWORK` environment variable (if set)
/// 3. Default network (Signet)
pub(super) fn resolve_network(arg: Option<&str>) -> anyhow::Result<Network> {
    // First, check if a command-line argument was provided
    if let Some(network_str) = arg {
        return match network_str {
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            n => anyhow::bail!("unsupported network option: {n}"),
        };
    }

    // If no argument provided, check environment variable
    if let Ok(env_network) = env::var(BITCOIN_NETWORK_ENVVAR) {
        return match env_network.as_str() {
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            n => anyhow::bail!("unsupported network option in {BITCOIN_NETWORK_ENVVAR}: {n}"),
        };
    }

    // Fall back to default
    Ok(DEFAULT_NETWORK)
}

/// Executes a `gen*` subcommand.
pub(super) fn exec_subc(cmd: Subcommand, ctx: &mut CmdContext) -> anyhow::Result<()> {
    match cmd {
        Subcommand::Xpriv(subc) => exec_genxpriv(subc, ctx),
        Subcommand::SeqPubkey(subc) => exec_genseqpubkey(subc, ctx),
        Subcommand::SeqPrivkey(subc) => exec_genseqprivkey(subc, ctx),
        Subcommand::Params(subc) => exec_genparams(subc, ctx),
        #[cfg(feature = "btc-client")]
        Subcommand::GenL1View(subc) => exec_genl1view(subc, ctx),
    }
}

/// Exports an ELF file to the specified path.
///
/// When the `sp1` feature is enabled, uses `strata_sp1_guest_builder` for the export.
///
/// # Arguments
///
/// * `elf_path` - The destination path for the ELF file.
///
/// # Errors
///
/// Returns an error if the export process fails.
fn export_elf(_elf_path: &Path) -> anyhow::Result<()> {
    #[cfg(feature = "sp1-builder")]
    {
        strata_sp1_guest_builder::export_elf(_elf_path)?
    }

    Ok(())
}

/// Returns the appropriate [`PredicateKey`] based on the enabled features.
///
/// # Behavior
///
/// - If the **sp1** feature is enabled, returns an
///   [Sp1Groth16](strata_predicate::PredicateTypeId::Sp1Groth16) PredicateKey.
/// - If **sp1** is not enabled, returns a
///   [AlwaysAccept](strata_predicate::PredicateTypeId::AlwaysAccept) PredicateKey.
fn resolve_checkpoint_predicate() -> PredicateKey {
    // Use SP1 if `sp1` feature is enabled
    #[cfg(feature = "sp1-builder")]
    {
        use strata_predicate::PredicateTypeId;
        use strata_sp1_guest_builder::GUEST_CHECKPOINT_VK_HASH_STR;
        use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;
        let vk_buf32: Buf32 = GUEST_CHECKPOINT_VK_HASH_STR
            .parse()
            .expect("invalid sp1 checkpoint verifier key hash");
        let sp1_verifier = SP1Groth16Verifier::load(&sp1_verifier::GROTH16_VK_BYTES, vk_buf32.0)
            .expect("Failed to load SP1 Groth16 verifier");
        let condition_bytes = sp1_verifier.vk.to_uncompressed_bytes();
        PredicateKey::new(PredicateTypeId::Sp1Groth16, condition_bytes)
    }

    // If `sp1` is not enabled, use the AlwaysAccept predicate
    #[cfg(not(feature = "sp1-builder"))]
    {
        PredicateKey::always_accept()
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

    let result = fs::write(&cmd.path, xpriv.to_string().as_bytes());

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
    println!("{seq_xpub}");

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
    println!("{seq_xpriv}");

    Ok(())
}

/// Executes the `genl1view` subcommand.
///
/// Fetches the genesis L1 view from a Bitcoin node at the specified height.
#[cfg(feature = "btc-client")]
fn exec_genl1view(cmd: SubcGenL1View, ctx: &mut CmdContext) -> anyhow::Result<()> {
    use tokio::runtime;

    use crate::btc_client::fetch_genesis_l1_view_with_config;

    let config = ctx
        .bitcoind_config
        .as_ref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Bitcoin RPC configuration not provided. Please specify --bitcoin-rpc-url, --bitcoin-rpc-user, and --bitcoin-rpc-password"
            )
        })?;

    let gl1view = runtime::Runtime::new()?.block_on(fetch_genesis_l1_view_with_config(
        config,
        cmd.genesis_l1_height,
    ))?;

    let params_buf = serde_json::to_string_pretty(&gl1view)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

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
            let xpub = Xpub::from_str(seqkey)?;
            Some(Buf32(xpub.to_x_only_pub().serialize()))
        }
        None => None,
    };

    // Get genesis L1 view first (before moving other fields)
    let genesis_l1_view = retrieve_genesis_l1_view(&cmd, ctx)?;

    // Parse each of the operator keys.
    let mut opkeys = Vec::new();

    if let Some(opkeys_path) = cmd.opkeys {
        let opkeys_str = fs::read_to_string(opkeys_path)?;

        for line in opkeys_str.lines() {
            // skip lines that are empty or look like comments
            if line.trim().is_empty() || line.starts_with("#") {
                continue;
            }

            opkeys.push(Xpriv::from_str(line)?);
        }
    }

    for key in cmd.opkey {
        opkeys.push(Xpriv::from_str(&key)?);
    }

    // Parse the deposit size str.
    let deposit_sats = cmd
        .deposit_sats
        .map(|s| parse_abbr_amt(&s))
        .transpose()?
        .unwrap_or(1_000_000_000);

    // Parse the checkpoint verification key.
    let rollup_vk = resolve_checkpoint_predicate();

    let chainspec_json = match cmd.chain_config {
        Some(path) => fs::read_to_string(path)?,
        None => DEFAULT_CHAIN_SPEC.into(),
    };

    let evm_genesis_info = get_alpen_ee_genesis_block_info(&chainspec_json)?;

    let magic: MagicBytes = if let Some(name_str) = &cmd.name {
        name_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid magic bytes: {}", e))?
    } else {
        "alpn".parse().expect("default magic bytes should be valid")
    };

    let config = ParamsConfig {
        magic,
        bitcoin_network: ctx.bitcoin_network,
        genesis_l1_view,
        // TODO make these consts
        block_time_sec: cmd.block_time.unwrap_or(15),
        epoch_slots: cmd.epoch_slots.unwrap_or(64),
        seqkey,
        opkeys,
        checkpoint_predicate: rollup_vk,
        // TODO make a const
        deposit_sats,
        proof_timeout: cmd.proof_timeout,
        evm_genesis_info,
    };

    let params = match construct_params(config) {
        Ok(p) => p,
        Err(e) => anyhow::bail!("failed to construct params: {e}"),
    };
    let params_buf = serde_json::to_string_pretty(&params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    if let Some(elf_path) = &cmd.elf_dir {
        export_elf(elf_path)?;
    }

    Ok(())
}

/// Generates a new [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`].
///
/// # Notes
///
/// Takes a mutable reference to an RNG to allow flexibility in testing.
/// The actual generation requires a high-entropy source like [`OsRng`](rand_core::OsRng)
/// to securely generate extended private keys.
fn gen_priv<R: CryptoRngCore>(rng: &mut R, net: Network) -> ZeroizableXpriv {
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
fn read_xpriv(path: &Path) -> anyhow::Result<Xpriv> {
    let xpriv = Xpriv::from_str(&fs::read_to_string(path)?)?;
    Ok(xpriv)
}

/// Parses an [`Xpriv`] from environment variable.
fn parse_xpriv_from_env(env: &'static str) -> anyhow::Result<Xpriv> {
    let env_val = match env::var(env) {
        Ok(v) => v,
        Err(_) => anyhow::bail!("got --key-from-env but {env} not set or invalid"),
    };

    let xpriv = match Xpriv::from_str(&env_val) {
        Ok(xpriv) => xpriv,
        Err(_) => anyhow::bail!("got --key-from-env but invalid xpriv"),
    };

    Ok(xpriv)
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
) -> anyhow::Result<Option<Xpriv>> {
    match (path, from_env) {
        (Some(_), true) => anyhow::bail!("got key path and --key-from-env, pick a lane"),
        (Some(path), false) => Ok(Some(read_xpriv(path)?)),
        (None, true) => parse_xpriv_from_env(env).map(Some),
        _ => Ok(None),
    }
}

/// Inputs for constructing the network parameters.
pub(crate) struct ParamsConfig {
    /// Name of the network.
    magic: MagicBytes,
    /// Network to use.
    bitcoin_network: Network,
    /// Block time in seconds.
    block_time_sec: u64,
    /// Number of slots in an epoch.
    epoch_slots: u32,
    /// View of the L1 at genesis
    genesis_l1_view: GenesisL1View,
    /// Sequencer's key.
    seqkey: Option<Buf32>,
    /// Operators' master keys.
    opkeys: Vec<Xpriv>,
    /// Verifier's key.
    checkpoint_predicate: PredicateKey,
    /// Amount of sats to deposit.
    deposit_sats: u64,
    /// Timeout for proofs.
    proof_timeout: Option<u32>,
    /// evm chain config json.
    evm_genesis_info: BlockInfo,
}

/// Constructs the parameters for a Strata network.
// TODO convert this to also initialize the sync params
fn construct_params(config: ParamsConfig) -> Result<RollupParams, KeyError> {
    let cr = config
        .seqkey
        .map(block_credential::CredRule::SchnorrKey)
        .unwrap_or(block_credential::CredRule::Unchecked);

    let opkeys: Vec<XOnlyPublicKey> = config
        .opkeys
        .iter()
        .map(|o| o.to_keypair(SECP256K1).x_only_public_key().0)
        .collect();

    Ok(RollupParams {
        magic_bytes: config.magic,
        block_time: config.block_time_sec * 1000,
        cred_rule: cr,
        // TODO do we want to remove this?
        genesis_l1_view: config.genesis_l1_view,
        operators: opkeys,
        evm_genesis_block_hash: config.evm_genesis_info.blockhash.0.into(),
        evm_genesis_block_state_root: config.evm_genesis_info.stateroot.0.into(),
        // TODO make configurable
        l1_reorg_safe_depth: 4,
        target_l2_batch_size: config.epoch_slots as u64,
        deposit_amount: Amount::from_sat(config.deposit_sats),
        checkpoint_predicate: config.checkpoint_predicate,
        // TODO make configurable
        dispatch_assignment_dur: 64,
        recovery_delay: DEFAULT_RECOVERY_DELAY,
        proof_publish_mode: config
            .proof_timeout
            .map(|t| ProofPublishMode::Timeout(t as u64))
            .unwrap_or(ProofPublishMode::Strict),
        // TODO make configurable
        max_deposits_in_block: 16,
        network: config.bitcoin_network,
    })
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

struct BlockInfo {
    blockhash: B256,
    stateroot: B256,
}

fn get_alpen_ee_genesis_block_info(genesis_json: &str) -> anyhow::Result<BlockInfo> {
    let genesis: Genesis = serde_json::from_str(genesis_json)?;

    let chain_spec = ChainSpec::from_genesis(genesis);

    let genesis_header = chain_spec.genesis_header();
    let genesis_stateroot = genesis_header.state_root;
    let genesis_hash = chain_spec.genesis_hash();

    Ok(BlockInfo {
        blockhash: genesis_hash,
        stateroot: genesis_stateroot,
    })
}

/// Retrieves the genesis L1 view from a file or Bitcoin RPC client.
///
/// This function follows a priority order:
/// 1. If `genesis_l1_view_file` is provided, load the genesis L1 view from that JSON file
/// 2. If no file is provided and the `btc-client` feature is enabled, fetch the genesis L1 view
///    from a Bitcoin node using the RPC credentials at the specified block height (defaults to
///    [`DEFAULT_L1_GENESIS_HEIGHT`] if not provided)
/// 3. If neither file nor Bitcoin client are available, return an error
///
/// # Arguments
/// * `cmd` - Command parameters containing file path and Bitcoin RPC connection details
/// * `ctx` - Command context containing the Bitcoin client (when btc-client feature is enabled)
///
/// # Returns
/// * `Ok(GenesisL1View)` - The successfully retrieved genesis L1 view
/// * `Err(anyhow::Error)` - If file reading fails, RPC connection fails, or neither option is
///   available
fn retrieve_genesis_l1_view(cmd: &SubcParams, ctx: &CmdContext) -> anyhow::Result<GenesisL1View> {
    // Priority 1: Use file if provided
    if let Some(ref file) = cmd.genesis_l1_view_file {
        let content = fs::read_to_string(file).map_err(|e| {
            anyhow::anyhow!("Failed to read genesis L1 view file {:?}: {}", file, e)
        })?;

        let genesis_l1_view: GenesisL1View = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse genesis L1 view JSON: {}", e))?;

        return Ok(genesis_l1_view);
    }

    // Priority 2: Use Bitcoin client if available
    #[cfg(feature = "btc-client")]
    {
        use crate::btc_client::fetch_genesis_l1_view_with_config;

        if let Some(config) = &ctx.bitcoind_config {
            use tokio::runtime;

            return runtime::Runtime::new()?.block_on(fetch_genesis_l1_view_with_config(
                config,
                cmd.genesis_l1_height.unwrap_or(DEFAULT_L1_GENESIS_HEIGHT),
            ));
        }
    }

    // Priority 3: Return error if neither option is available
    Err(anyhow::anyhow!(
        "Either provide --genesis-l1-view-file or specify Bitcoin RPC credentials (--bitcoin-rpc-url, --bitcoin-rpc-user, --bitcoin-rpc-password) when btc-client feature is enabled"
    ))
}

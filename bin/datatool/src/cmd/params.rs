//! `genparams` subcommand: generates network params from inputs.

use std::{fs, path::Path, str::FromStr};

use alloy_genesis::Genesis;
use alloy_primitives::B256;
use bitcoin::{
    bip32::{Xpriv, Xpub},
    secp256k1::SECP256K1,
    Amount, XOnlyPublicKey,
};
use reth_chainspec::ChainSpec;
use strata_key_derivation::error::KeyError;
use strata_l1_txfmt::MagicBytes;
use strata_params::{ProofPublishMode, RollupParams};
use strata_predicate::PredicateKey;
use strata_primitives::{block_credential, buf::Buf32, l1::GenesisL1View};

use crate::{
    args::{CmdContext, SubcParams},
    util::parse_abbr_amt,
};

/// The default L1 genesis height to use.
const DEFAULT_L1_GENESIS_HEIGHT: u64 = 100;

/// The default evm chainspec to use in params.
const DEFAULT_CHAIN_SPEC: &str = alpen_chainspec::DEV_CHAIN_SPEC;

/// The default recovery delay to use in params.
const DEFAULT_RECOVERY_DELAY: u16 = 1_008;

/// The default block time in seconds to use in params.
const DEFAULT_BLOCK_TIME_SEC: u64 = 5;

/// The default epoch slots to use in params.
const DEFAULT_EPOCH_SLOTS: u32 = 64;

/// Executes the `genparams` subcommand.
///
/// Generates the params for a Strata network.
/// Either writes to a file or prints to stdout depending on the provided options.
pub(super) fn exec(cmd: SubcParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
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
        block_time_sec: cmd.block_time.unwrap_or(DEFAULT_BLOCK_TIME_SEC),
        epoch_slots: cmd.epoch_slots.unwrap_or(DEFAULT_EPOCH_SLOTS),
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

/// Exports an ELF file to the specified path.
///
/// When the `sp1` feature is enabled, uses `strata_sp1_guest_builder` for the export.
fn export_elf(_elf_path: &Path) -> anyhow::Result<()> {
    #[cfg(feature = "sp1-builder")]
    {
        strata_sp1_guest_builder::export_elf(_elf_path)?
    }

    Ok(())
}

/// Returns the appropriate [`PredicateKey`] based on the enabled features.
///
/// If the **sp1** feature is enabled, returns an Sp1Groth16 PredicateKey.
/// Otherwise, returns an AlwaysAccept PredicateKey.
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

/// Inputs for constructing the network parameters.
struct ParamsConfig {
    /// Name of the network.
    magic: MagicBytes,
    /// Network to use.
    bitcoin_network: bitcoin::Network,
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
/// Priority:
/// 1. If `genesis_l1_view_file` is provided, load from that JSON file
/// 2. If `btc-client` feature is enabled and RPC credentials are available, fetch from Bitcoin node
/// 3. Otherwise, return an error
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

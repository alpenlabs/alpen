//! Utility functions for Strata `datatool` binary.
//!
//! It contains functions for generating keys, parsing amounts, and constructing
//! network parameters.
//! These functions are called from the CLI's subcommands.

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use alloy_genesis::Genesis;
use alloy_primitives::B256;
use bitcoin::{
    bip32::{Xpriv, Xpub},
    secp256k1::SECP256K1,
    CompactTarget, Network,
};
use bitcoind_async_client::{traits::Reader, Client};
use rand_core::CryptoRngCore;
use reth_chainspec::ChainSpec;
use shrex::Hex;
use strata_key_derivation::{error::KeyError, operator::OperatorKeys, sequencer::SequencerKeys};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::{
    block_credential,
    buf::Buf32,
    crypto::EvenSecretKey,
    keys::ZeroizableXpriv,
    l1::{
        get_relative_difficulty_adjustment_height, BtcParams, L1BlockCommitment, L1BlockId,
        TIMESTAMPS_FOR_MEDIAN,
    },
    operator::OperatorPubkeys,
    params::{GenesisL1View, ProofPublishMode, RollupParams},
    proof::RollupVerifyingKey,
};
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

/// The default evm chainspec to use in params.
const DEFAULT_CHAIN_SPEC: &str = alpen_chainspec::DEV_CHAIN_SPEC;

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
pub(super) async fn exec_subc(cmd: Subcommand, ctx: &mut CmdContext) -> anyhow::Result<()> {
    match cmd {
        Subcommand::Xpriv(subc) => exec_genxpriv(subc, ctx),
        Subcommand::SeqPubkey(subc) => exec_genseqpubkey(subc, ctx),
        Subcommand::SeqPrivkey(subc) => exec_genseqprivkey(subc, ctx),
        Subcommand::OpXpub(subc) => exec_genopxpub(subc, ctx),
        Subcommand::Params(subc) => exec_genparams(subc, ctx).await,
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
#[allow(clippy::missing_const_for_fn)]
fn export_elf(_elf_path: &Path) -> anyhow::Result<()> {
    #[cfg(feature = "sp1-builder")]
    {
        strata_sp1_guest_builder::export_elf(_elf_path)?
    }

    Ok(())
}

/// Returns the appropriate [`RollupVerifyingKey`] based on the enabled features.
///
/// # Behavior
///
/// - If the **sp1** feature is exclusively enabled, returns an `SP1VerifyingKey`.
/// - If the **risc0** feature is exclusively enabled, returns a `Risc0VerifyingKey`.
/// - If **both** `sp1` and `risc0` are enabled at once, this function will **panic**.
/// - If **neither** `sp1` nor `risc0` is enabled, returns a `NativeVerifyingKey`.
///
/// # Panics
///
/// Panics if both `sp1` and `risc0` features are enabled simultaneously, since
/// only one ZKVM can be supported at a time.
fn resolve_rollup_vk() -> RollupVerifyingKey {
    // Use SP1 if only `sp1` feature is enabled
    #[cfg(all(feature = "sp1-builder", not(feature = "risc0-builder")))]
    {
        use strata_sp1_guest_builder::GUEST_CHECKPOINT_VK_HASH_STR;
        use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;
        let vk_buf32: Buf32 = GUEST_CHECKPOINT_VK_HASH_STR
            .parse()
            .expect("invalid sp1 checkpoint verifier key hash");
        let sp1_verifier = SP1Groth16Verifier::load(&sp1_verifier::GROTH16_VK_BYTES, vk_buf32.0)
            .expect("Failed to load SP1 Groth16 verifier");
        RollupVerifyingKey::SP1VerifyingKey(sp1_verifier)
    }

    // Use Risc0 if only `risc0` feature is enabled
    #[cfg(all(feature = "risc0-builder", not(feature = "sp1-builder")))]
    {
        use strata_risc0_guest_builder::GUEST_RISC0_CHECKPOINT_ID;
        use zkaleido_risc0_groth16_verifier::Risc0Groth16Verifier;
        let risc0_verifier = Risc0Groth16Verifier::load(
            risc0_groth16::verifying_key(),
            risc0_zkvm::BN254_IDENTITY_CONTROL_ID,
            risc0_zkvm::ALLOWED_CONTROL_ROOT,
            Digest::new(GUEST_RISC0_CHECKPOINT_ID),
        )
        .expect("Failed to load Risc0 Groth16 verifier");
        RollupVerifyingKey::Risc0VerifyingKey(risc0_verifier)
    }

    // Panic if both `sp1` and `risc0` feature are enabled
    #[cfg(all(feature = "risc0-builder", feature = "sp1-builder"))]
    {
        panic!(
            "Conflicting ZKVM features: both 'sp1' and 'risc0' are enabled. \
             Please disable one of them, as only a single ZKVM can be supported at a time."
        )
    }

    // If neither `risc0` nor `sp1` is enabled, use the Native verifying key
    #[cfg(all(not(feature = "risc0-builder"), not(feature = "sp1-builder")))]
    {
        RollupVerifyingKey::NativeVerifyingKey
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

/// Executes the `genopxpub` subcommand.
///
/// Generates the root xpub for an operator.
fn exec_genopxpub(cmd: SubcOpXpub, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, OPKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let op_keys = OperatorKeys::new(&xpriv)?;
    if cmd.p2p {
        let p2p_pk = EvenSecretKey::from(op_keys.message_xpriv().private_key)
            .x_only_public_key(SECP256K1)
            .0;
        println!("{:?}", Hex(p2p_pk.serialize()));
    }

    if cmd.wallet {
        let wallet_pk = EvenSecretKey::from(op_keys.wallet_xpriv().private_key)
            .x_only_public_key(SECP256K1)
            .0;
        println!("{:?}", Hex(wallet_pk.serialize()));
    }

    Ok(())
}

/// Executes the `genparams` subcommand.
///
/// Generates the params for a Strata network.
/// Either writes to a file or prints to stdout depending on the provided options.
async fn exec_genparams(cmd: SubcParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    // Parse the sequencer key, trimming whitespace for convenience.
    let seqkey = match cmd.seqkey.as_ref().map(|s| s.trim()) {
        Some(seqkey) => {
            let xpub = Xpub::from_str(seqkey)?;
            Some(Buf32(xpub.to_x_only_pub().serialize()))
        }
        None => None,
    };

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
    let rollup_vk = resolve_rollup_vk();

    let chainspec_json = match cmd.chain_config {
        Some(path) => fs::read_to_string(path)?,
        None => DEFAULT_CHAIN_SPEC.into(),
    };

    let evm_genesis_info = get_genesis_block_info(&chainspec_json)?;

    // Create Bitcoin RPC client and fetch genesis L1 view
    let bitcoin_client = Client::new(
        cmd.bitcoin_rpc_url,
        cmd.bitcoin_rpc_user,
        cmd.bitcoin_rpc_password,
        None,
        None,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Bitcoin RPC client: {}", e))?;

    let genesis_l1_view =
        fetch_genesis_l1_view(&bitcoin_client, cmd.genesis_l1_height.unwrap_or(100)).await?;

    let magic: MagicBytes = if let Some(name_str) = &cmd.name {
        // Validate that the name is ASCII
        if !name_str.is_ascii() {
            return Err(anyhow::anyhow!("Name must contain only ASCII characters"));
        }

        // Validate that the name is exactly 4 bytes
        let name_bytes = name_str.as_bytes();
        if name_bytes.len() != 4 {
            return Err(anyhow::anyhow!(
                "Name must be exactly 4 bytes long, got {}",
                name_bytes.len()
            ));
        }

        let mut magic_bytes: [u8; 4] = [0; 4];
        magic_bytes.copy_from_slice(name_bytes);
        magic_bytes
    } else {
        *b"alpn"
    };

    let config = ParamsConfig {
        magic,
        checkpoint_tag: cmd.checkpoint_tag.unwrap_or("strata-ckpt".to_string()),
        da_tag: cmd.da_tag.unwrap_or("strata-da".to_string()),
        bitcoin_network: ctx.bitcoin_network,
        genesis_l1_view,
        // TODO make these consts
        block_time_sec: cmd.block_time.unwrap_or(15),
        epoch_slots: cmd.epoch_slots.unwrap_or(64),
        seqkey,
        opkeys,
        rollup_vk,
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
    let env_val = match std::env::var(env) {
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
    /// Tagname used to identify DA envelopes
    da_tag: String,
    /// Tagname used to identify Checkpoint envelopes
    checkpoint_tag: String,
    /// Network to use.
    #[allow(unused)]
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
    rollup_vk: RollupVerifyingKey,
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

    let opkeys = config
        .opkeys
        .iter()
        .map(OperatorKeys::new)
        .collect::<Result<Vec<_>, _>>()?;

    let pub_opkeys = opkeys.iter().map(|keys| {
        OperatorPubkeys::new(
            EvenSecretKey::from(keys.message_xpriv().private_key)
                .x_only_public_key(SECP256K1)
                .0
                .serialize()
                .into(),
            EvenSecretKey::from(keys.wallet_xpriv().private_key)
                .x_only_public_key(SECP256K1)
                .0
                .serialize()
                .into(),
        )
    });

    Ok(RollupParams {
        magic_bytes: config.magic,
        block_time: config.block_time_sec * 1000,
        da_tag: config.da_tag,
        checkpoint_tag: config.checkpoint_tag,
        cred_rule: cr,
        // TODO do we want to remove this?
        genesis_l1_view: config.genesis_l1_view,
        operator_config: strata_primitives::params::OperatorConfig::Static(pub_opkeys.collect()),
        evm_genesis_block_hash: config.evm_genesis_info.blockhash.0.into(),
        evm_genesis_block_state_root: config.evm_genesis_info.stateroot.0.into(),
        // TODO make configurable
        l1_reorg_safe_depth: 4,
        target_l2_batch_size: config.epoch_slots as u64,
        address_length: 20,
        deposit_amount: config.deposit_sats,
        rollup_vk: config.rollup_vk,
        // TODO make configurable
        dispatch_assignment_dur: 64,
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

fn get_genesis_block_info(genesis_json: &str) -> anyhow::Result<BlockInfo> {
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

async fn fetch_genesis_l1_view(
    client: &impl Reader,
    block_height: u64,
) -> anyhow::Result<GenesisL1View> {
    // Create BTC parameters based on the current network.
    let network = client.network().await?;
    let btc_params = BtcParams::from(bitcoin::params::Params::from(network));

    // Get the difficulty adjustment block just before the given block height,
    // representing the start of the current epoch.
    let current_epoch_start_height =
        get_relative_difficulty_adjustment_height(0, block_height, btc_params.inner());
    let current_epoch_start_header = client
        .get_block_header_at(current_epoch_start_height)
        .await?;

    // Fetch the block header at the height
    let block_header = client.get_block_header_at(block_height).await?;

    // Fetch timestamps
    let timestamps =
        fetch_block_timestamps_ascending(client, block_height, TIMESTAMPS_FOR_MEDIAN).await?;
    let timestamps: [u32; TIMESTAMPS_FOR_MEDIAN] = timestamps.try_into().expect(
        "fetch_block_timestamps_ascending should return exactly TIMESTAMPS_FOR_MEDIAN timestamps",
    );

    // Compute the block ID for the verified block.
    let block_id: L1BlockId = block_header.block_hash().into();

    // If (block_height + 1) is the start of the new epoch, we need to calculate the
    // next_block_target, else next_block_target will be current block's target
    let next_block_target =
        if (block_height + 1).is_multiple_of(btc_params.difficulty_adjustment_interval()) {
            CompactTarget::from_next_work_required(
                block_header.bits,
                (block_header.time - current_epoch_start_header.time) as u64,
                &btc_params,
            )
            .to_consensus()
        } else {
            client
                .get_block_header_at(block_height)
                .await?
                .target()
                .to_compact_lossy()
                .to_consensus()
        };

    // Build the genesis L1 veiw structure.
    let genesis_l1_view = GenesisL1View {
        blk: L1BlockCommitment::new(block_height, block_id),
        next_target: next_block_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        last_11_timestamps: timestamps,
    };

    Ok(genesis_l1_view)
}

/// Retrieves the timestamps for a specified number of blocks starting from the given block height,
/// moving backwards. For each block from `height` down to `height - count + 1`, it fetches the
/// block’s timestamp. If a block height is less than 1 (i.e. there is no block), it inserts a
/// placeholder value of 0. The resulting vector is then reversed so that timestamps are returned in
/// ascending order (oldest first).
async fn fetch_block_timestamps_ascending(
    client: &impl Reader,
    height: u64,
    count: usize,
) -> anyhow::Result<Vec<u32>> {
    let mut timestamps = Vec::with_capacity(count);

    for i in 0..count {
        let current_height = height.saturating_sub(i as u64);
        // If we've gone past block 1, push 0 as a placeholder.
        if current_height < 1 {
            timestamps.push(0);
        } else {
            let header = client.get_block_header_at(current_height).await?;
            timestamps.push(header.time);
        }
    }

    timestamps.reverse();
    Ok(timestamps)
}

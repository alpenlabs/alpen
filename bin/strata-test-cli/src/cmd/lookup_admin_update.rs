//! Recover an accepted update's ID by replaying Bitcoin's admin history.

use std::{
    any::Any,
    fs,
    io::{self, Write},
    path::PathBuf,
};

use anyhow::{bail, ensure, Context};
use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client, RpcApi};
use bitcoin::{hashes::Hash, Block, Txid};
use serde_json::{json, Value};
use strata_asm_common::{
    AsmHistoryAccumulatorState, AsmLogEntry, AuxData, HeaderVerificationState, InterprotoMsg,
    MsgRelayer, Subprotocol, TxInputRef, VerifiedAuxData,
};
use strata_asm_logs::AsmStfUpdate;
use strata_asm_params::AsmParams;
use strata_asm_proto_admin::{AdministrationSubprotoState, AdministrationSubprotocol};
use strata_asm_proto_admin_txs::{
    actions::{MultisigAction, UpdateAction},
    parser::parse_tx,
};
use strata_asm_stf::group_txs_by_subprotocol;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::{L1BlockCommitment, L1BlockId, RBuf32};

/// Look up an admin reveal transaction using Bitcoin RPC and deployed genesis ASM parameters.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "lookup-admin-update")]
pub struct LookupAdminUpdateArgs {
    /// confirmed update reveal transaction ID
    #[argh(option)]
    pub txid: Txid,
    /// deployed genesis ASM parameters JSON
    #[argh(option)]
    pub asm_params: PathBuf,
    /// bitcoin RPC URL
    #[argh(option)]
    pub btc_url: String,
    /// bitcoin RPC user
    #[argh(option)]
    pub btc_user: String,
    /// bitcoin RPC password
    #[argh(option)]
    pub btc_password: String,
}

pub(crate) fn lookup_admin_update(args: LookupAdminUpdateArgs) -> Result<(), DisplayedError> {
    lookup(args).user_error("admin update lookup failed")
}

// Admin has no incoming interprotocol messages. Outbound messages affect other
// subprotocols, not its queue; retain logs to detect unsupported ASM rule changes.
#[derive(Default)]
struct ReplayOutput(Vec<AsmLogEntry>);

impl MsgRelayer for ReplayOutput {
    fn relay_msg(&mut self, _message: &dyn InterprotoMsg) {}
    fn emit_log(&mut self, log: AsmLogEntry) {
        self.0.push(log);
    }
    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

fn process(
    state: &mut AdministrationSubprotoState,
    txs: &[TxInputRef<'_>],
    header: &HeaderVerificationState,
    aux: &VerifiedAuxData,
) -> anyhow::Result<()> {
    let mut output = ReplayOutput::default();
    AdministrationSubprotocol::process_txs(state, txs, header, aux, &mut output);
    ensure!(
        !output.0.iter().any(|log| log.try_into_log::<AsmStfUpdate>().is_ok()),
        "history enacts an ASM STF update; this lookup cannot safely replay across ASM rule changes"
    );
    Ok(())
}

fn replay_block(
    state: &mut AdministrationSubprotoState,
    block: &Block,
    params: &AsmParams,
    header: &HeaderVerificationState,
    aux: &VerifiedAuxData,
    target: Txid,
) -> anyhow::Result<Option<(u32, UpdateAction)>> {
    let txs = group_txs_by_subprotocol(params.magic, block)
        .remove(&AdministrationSubprotocol::ID)
        .unwrap_or_default();
    let mut found = None;
    if let Some(index) = txs.iter().position(|tx| tx.tx().compute_txid() == target) {
        let payload = parse_tx(&txs[index]).context("target is not a valid admin payload")?;
        let MultisigAction::Update(action) = payload.action else {
            bail!("target is a cancellation, not an update");
        };
        let mut before = state.clone();
        process(&mut before, &txs[..index], header, aux)?;
        let mut after = state.clone();
        process(&mut after, &txs[..=index], header, aux)?;
        let id = before.next_update_id();
        ensure!(
            after.next_update_id() == id.checked_add(1).context("update ID overflow")?,
            "target transaction was rejected by the admin state machine"
        );
        found = Some((id, action));
    }
    process(state, &txs, header, aux)?;
    Ok(found)
}

fn lookup(args: LookupAdminUpdateArgs) -> anyhow::Result<()> {
    let params: AsmParams = serde_json::from_slice(
        &fs::read(&args.asm_params).context("read deployed genesis ASM parameters")?,
    )
    .context("decode ASM parameters")?;
    let client = Client::new(
        &args.btc_url,
        Auth::UserPass(args.btc_user, args.btc_password),
    )
    .context("connect to Bitcoin RPC")?;
    let chain = client
        .get_blockchain_info()
        .context("get Bitcoin chain information")?;
    ensure!(
        chain.chain == params.anchor.network,
        "Bitcoin network differs from ASM parameters"
    );
    let start = u64::from(params.anchor.block.height());
    ensure!(
        chain.blocks >= start,
        "Bitcoin node has not reached the ASM anchor"
    );
    let mut previous = client
        .get_block_hash(start)
        .context("fetch ASM anchor block hash")?;
    ensure!(
        L1BlockId::from(RBuf32(previous.to_byte_array())) == *params.anchor.block.blkid(),
        "ASM anchor does not match Bitcoin's canonical chain"
    );
    let mut state = AdministrationSubprotocol::init(
        params
            .admin_config()
            .context("ASM parameters have no admin config")?,
    );
    let mut header = HeaderVerificationState::init(params.anchor.clone());
    let aux = VerifiedAuxData::try_new(&AuxData::default(), &AsmHistoryAccumulatorState::new(0))
        .context("initialize admin replay auxiliary data")?;
    let mut found = None;
    for height in start + 1..=chain.blocks {
        let hash = client
            .get_block_hash(height)
            .with_context(|| format!("get block hash at {height}"))?;
        let block = client.get_block(&hash).with_context(|| {
            format!("read block {height}; replay requires unpruned blocks since the ASM anchor")
        })?;
        ensure!(
            block.block_hash() == hash && block.header.prev_blockhash == previous,
            "Bitcoin chain changed during replay; retry lookup"
        );
        header.last_verified_block = L1BlockCommitment::new(
            u32::try_from(height)?,
            L1BlockId::from(RBuf32(hash.to_byte_array())),
        );
        if let Some((id, action)) =
            replay_block(&mut state, &block, &params, &header, &aux, args.txid)?
        {
            ensure!(found.is_none(), "target transaction appears more than once");
            found = Some((id, action, height));
        }
        previous = hash;
        if height % 1000 == 0 {
            writeln!(
                io::stderr().lock(),
                "Scanned Bitcoin block {height}/{}",
                chain.blocks
            )?;
        }
    }
    ensure!(
        client.get_block_hash(chain.blocks)? == chain.best_block_hash
            && previous == chain.best_block_hash,
        "Bitcoin chain changed during replay; retry lookup"
    );
    let (id, action, height) =
        found.context("update reveal transaction not found after the ASM anchor")?;
    let queued = state.find_queued(&id);
    let predicate = match &action {
        UpdateAction::EeStfVk(update) => Some(update.key().to_string()),
        UpdateAction::OlStfVk(update) => Some(update.key().to_string()),
        UpdateAction::AsmStfVk(update) => Some(update.key().to_string()),
        _ => None,
    };
    let result: Value = json!({
        "txid": args.txid, "update_id": id, "confirmation_height": height,
        "update_type": action.update_tx_type().to_string(), "predicate": predicate,
        "authorized_by": action.required_role().to_string(),
        "last_seq_no": state.authority(action.required_role()).context("missing authority after replay")?.last_seqno(),
        "status": if queued.is_some() { "pending" } else { "not_pending" },
        "activation_height": queued.map(|update| update.activation_height()),
        "l1_height": chain.blocks, "l1_block_id": chain.best_block_hash,
    });
    writeln!(
        io::stdout().lock(),
        "{}",
        serde_json::to_string_pretty(&result)?
    )
    .context("write admin update lookup")
}

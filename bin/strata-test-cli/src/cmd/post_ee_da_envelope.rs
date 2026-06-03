//! Broadcasts crafted EE DA chunked envelopes for functional tests.

use std::str::FromStr;

use anyhow::{bail, Context};
use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::{json::ListUnspentResultEntry, Client, RpcApi};
use bitcoin::{
    consensus::encode::serialize_hex,
    key::Keypair,
    secp256k1::{SecretKey, SECP256K1},
    SignedAmount,
};
use bitcoind_async_client::corepc_types::model::ListUnspentItem;
use serde_json::json;
use strata_btcio::writer::{builder::EnvelopeConfig, chunked_envelope::build_chunked_envelope_txs};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_txfmt::MagicBytes;

use crate::{constants::NETWORK, taproot::new_bitcoind_client};

const DEFAULT_REVEAL_AMOUNT_SATS: u64 = 546;
const CRAFTED_DA_BLOB_VERSION: u32 = 0;
const TEST_ENVELOPE_SECRET_KEY: [u8; 32] = [42; 32];

/// Broadcast a crafted EE DA chunked envelope for functional tests.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "post-ee-da-envelope")]
pub struct PostEeDaEnvelopeArgs {
    /// bitcoind RPC URL, without credentials.
    #[argh(option)]
    pub bitcoind_url: String,

    /// bitcoind RPC username.
    #[argh(option)]
    pub rpc_user: String,

    /// bitcoind RPC password.
    #[argh(option)]
    pub rpc_password: String,

    /// optional wallet name to append to the RPC URL.
    #[argh(option)]
    pub wallet_name: Option<String>,

    /// DA magic bytes used in the commit `OP_RETURN`.
    #[argh(option)]
    pub magic_bytes: MagicBytes,

    /// fee rate in sats/vByte.
    #[argh(option)]
    pub fee_rate: u64,

    /// how many reveal txs to broadcast, starting at chunk 0.
    #[argh(option)]
    pub reveal_count: Option<usize>,

    /// mining behavior after broadcast: inline or manual.
    ///
    /// `inline` mines the commit in one block, then broadcasts all selected
    /// reveals and mines them together in one subsequent block. With
    /// `--reveal-count=0`, only the commit block is mined.
    #[argh(option, default = "MineMode::Inline")]
    pub mine_mode: MineMode,

    /// hex-encoded chunk payload; repeat once per chunk.
    #[argh(option)]
    pub chunk_hex: Vec<HexBytes>,
}

/// Mining mode for crafted-envelope publication.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum MineMode {
    /// Mine the commit and selected reveals inside the command.
    Inline,

    /// Broadcast selected txs only; the test driver mines blocks.
    Manual,
}

impl FromStr for MineMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "inline" => Ok(Self::Inline),
            "manual" => Ok(Self::Manual),
            other => Err(format!(
                "unknown mine mode {other:?}; expected inline or manual"
            )),
        }
    }
}

/// Hex-encoded byte string.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct HexBytes(pub Vec<u8>);

impl FromStr for HexBytes {
    type Err = hex::FromHexError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        hex::decode(value.trim_start_matches("0x")).map(Self)
    }
}

pub(crate) fn post_ee_da_envelope(args: PostEeDaEnvelopeArgs) -> Result<(), DisplayedError> {
    let output = build_and_post_ee_da_envelope(args).internal_error("post EE DA envelope")?;
    println!("{output}");
    Ok(())
}

fn build_and_post_ee_da_envelope(args: PostEeDaEnvelopeArgs) -> anyhow::Result<String> {
    if args.chunk_hex.is_empty() {
        bail!("--chunk-hex must be provided at least once");
    }

    let reveal_count = args.reveal_count.unwrap_or(args.chunk_hex.len());
    if reveal_count > args.chunk_hex.len() {
        bail!(
            "--reveal-count ({reveal_count}) cannot exceed chunk count ({})",
            args.chunk_hex.len()
        );
    }

    let client = new_bitcoind_client(
        &wallet_rpc_url(&args.bitcoind_url, args.wallet_name.as_deref()),
        None,
        Some(&args.rpc_user),
        Some(&args.rpc_password),
    )
    .context("create bitcoind RPC client")?;

    let change_addr = client
        .get_new_address(None, None)
        .context("get change address")?
        .require_network(NETWORK)
        .context("change address network mismatch")?;

    let config = EnvelopeConfig::new(
        args.magic_bytes,
        change_addr,
        NETWORK,
        args.fee_rate,
        DEFAULT_REVEAL_AMOUNT_SATS,
        None,
    );

    let utxos = client
        .list_unspent(None, None, None, None, None)
        .context("list unspent")?
        .into_iter()
        .map(to_corepc_list_unspent_item)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let chunks: Vec<Vec<u8>> = args.chunk_hex.into_iter().map(|h| h.0).collect();
    let secret_key = SecretKey::from_slice(&TEST_ENVELOPE_SECRET_KEY)
        .context("invalid test envelope secret key")?;
    let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
    let txs = build_chunked_envelope_txs(
        &config,
        &chunks,
        &config.magic_bytes,
        CRAFTED_DA_BLOB_VERSION,
        &keypair,
        utxos,
    )
    .context("build chunked envelope txs")?;

    let signed_commit = client
        .sign_raw_transaction_with_wallet(&txs.commit_tx, None, None)
        .context("sign commit")?
        .transaction()
        .context("decode signed commit")?;
    let commit_txid = client
        .send_raw_transaction(&signed_commit)
        .context("broadcast commit")?;

    if args.mine_mode == MineMode::Inline {
        mine_one_block(&client).context("mine commit block")?;
    }

    let mut broadcast_reveal_txids = Vec::with_capacity(reveal_count);
    for reveal_tx in txs.reveal_txs.iter().take(reveal_count) {
        let reveal_txid = client
            .send_raw_transaction(reveal_tx)
            .context("broadcast reveal")?;
        broadcast_reveal_txids.push(reveal_txid);
    }

    if args.mine_mode == MineMode::Inline && reveal_count > 0 {
        mine_one_block(&client).context("mine reveal block")?;
    }

    let reveal_txs: Vec<_> = txs
        .reveal_txs
        .iter()
        .enumerate()
        .map(|(index, tx)| {
            json!({
                "index": index,
                "txid": tx.compute_txid().to_string(),
                "wtxid": tx.compute_wtxid().to_string(),
                "hex": serialize_hex(tx),
                "broadcast": index < reveal_count,
            })
        })
        .collect();

    Ok(json!({
        "commit_txid": commit_txid.to_string(),
        "commit_wtxid": signed_commit.compute_wtxid().to_string(),
        "commit_hex": serialize_hex(&signed_commit),
        "broadcast_reveal_txids": broadcast_reveal_txids
            .into_iter()
            .map(|txid| txid.to_string())
            .collect::<Vec<_>>(),
        "reveal_txs": reveal_txs,
    })
    .to_string())
}

fn wallet_rpc_url(bitcoind_url: &str, wallet_name: Option<&str>) -> String {
    match wallet_name {
        Some(wallet_name) => {
            let base = bitcoind_url.trim_end_matches('/');
            format!("{base}/wallet/{wallet_name}")
        }
        None => bitcoind_url.to_owned(),
    }
}

fn mine_one_block(client: &Client) -> anyhow::Result<()> {
    let mine_addr = client
        .get_new_address(None, None)?
        .require_network(NETWORK)
        .context("mining address network mismatch")?;
    client.generate_to_address(1, &mine_addr)?;
    Ok(())
}

fn to_corepc_list_unspent_item(entry: ListUnspentResultEntry) -> anyhow::Result<ListUnspentItem> {
    let address = entry.address.context("listunspent entry has no address")?;
    Ok(ListUnspentItem {
        txid: entry.txid,
        vout: entry.vout,
        address,
        label: entry.label.unwrap_or_default(),
        script_pubkey: entry.script_pub_key,
        amount: SignedAmount::from_sat(entry.amount.to_sat() as i64),
        confirmations: entry.confirmations,
        redeem_script: entry.redeem_script,
        spendable: entry.spendable,
        solvable: entry.solvable,
        descriptor: entry.descriptor,
        safe: entry.safe,
        parent_descriptors: None,
    })
}

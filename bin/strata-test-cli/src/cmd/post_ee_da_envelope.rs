//! `post-ee-da-envelope`: broadcast one chunked envelope with caller-supplied
//! chunk bytes chained from a given predecessor wtxid.

use std::{
    io::{Error as IoError, ErrorKind},
    str::FromStr,
};

use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::RpcApi;
use bitcoin::Wtxid;
use strata_btcio::writer::{build_chunked_envelope_txs, builder::EnvelopeConfig};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::buf::Buf32;

use crate::{
    btc::{new_bitcoind_client, to_corepc_list_unspent_item},
    constants::NETWORK,
};

/// Broadcast one chunked envelope (commit + N reveals) to bitcoind.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "post-ee-da-envelope")]
pub struct PostEeDaEnvelopeArgs {
    /// bitcoind RPC URL (include `/wallet/<name>` for non-default wallets).
    #[argh(option)]
    pub bitcoind_url: String,

    /// bitcoind RPC username.
    #[argh(option)]
    pub rpc_user: String,

    /// bitcoind RPC password.
    #[argh(option)]
    pub rpc_password: String,

    /// DA linking-tag magic bytes (4 ASCII chars).
    #[argh(option)]
    pub magic_bytes: MagicBytes,

    /// fee rate in sats/vByte.
    #[argh(option)]
    pub fee_rate: u64,

    /// 32-byte predecessor wtxid as hex (all-zero for da-genesis).
    #[argh(option)]
    pub prev_wtxid: Buf32,

    /// hex-encoded chunk payload; repeat once per chunk (order preserved).
    #[argh(option)]
    pub chunk_hex: Vec<HexBytes>,
}

/// Hex-encoded byte string (with optional `0x` prefix) parsed directly by argh.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct HexBytes(pub Vec<u8>);

impl FromStr for HexBytes {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s.trim_start_matches("0x")).map(HexBytes)
    }
}

pub(crate) fn post_ee_da_envelope(args: PostEeDaEnvelopeArgs) -> Result<(), DisplayedError> {
    if args.chunk_hex.is_empty() {
        return Err(DisplayedError::UserError(
            "--chunk-hex must be provided at least once".to_string(),
            Box::new(IoError::new(ErrorKind::InvalidInput, "empty chunks")),
        ));
    }

    let client = new_bitcoind_client(
        &args.bitcoind_url,
        None,
        Some(&args.rpc_user),
        Some(&args.rpc_password),
    )
    .internal_error("create bitcoind rpc client")?;

    let change_addr = client
        .get_new_address(None, None)
        .internal_error("get_new_address for change")?
        .require_network(NETWORK)
        .internal_error("change address network mismatch")?;

    let config = EnvelopeConfig::new(
        args.magic_bytes,
        change_addr,
        NETWORK,
        args.fee_rate,
        546,
        None,
    );

    let utxos_raw = client
        .list_unspent(None, None, None, None, None)
        .internal_error("list_unspent")?;
    let utxos = utxos_raw
        .into_iter()
        .map(to_corepc_list_unspent_item)
        .collect::<Result<Vec<_>, _>>()
        .internal_error("convert UTXOs")?;

    let chunks: Vec<Vec<u8>> = args.chunk_hex.into_iter().map(|h| h.0).collect();

    let txs = build_chunked_envelope_txs(
        &config,
        &chunks,
        &config.magic_bytes,
        &args.prev_wtxid,
        utxos,
    )
    .internal_error("build chunked envelope txs")?;

    let signed_commit = client
        .sign_raw_transaction_with_wallet(&txs.commit_tx, None, None)
        .internal_error("sign commit")?
        .transaction()
        .internal_error("extract signed commit")?;
    client
        .send_raw_transaction(&signed_commit)
        .internal_error("broadcast commit")?;

    let mine_addr = client
        .get_new_address(None, None)
        .internal_error("mining address")?
        .require_network(NETWORK)
        .internal_error("mining address network mismatch")?;
    client
        .generate_to_address(1, &mine_addr)
        .internal_error("mine commit confirmation")?;

    let mut wtxids: Vec<Wtxid> = Vec::with_capacity(txs.reveal_txs.len());
    for reveal in &txs.reveal_txs {
        client
            .send_raw_transaction(reveal)
            .internal_error("broadcast reveal")?;
        wtxids.push(reveal.compute_wtxid());
    }
    client
        .generate_to_address(1, &mine_addr)
        .internal_error("mine reveal confirmation")?;

    for w in &wtxids {
        println!("{w}");
    }
    Ok(())
}

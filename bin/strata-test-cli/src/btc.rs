//! Bitcoin RPC client helpers shared across subcommands.

use std::path::PathBuf;

use bdk_bitcoind_rpc::bitcoincore_rpc::{json::ListUnspentResultEntry, Auth, Client};
use bitcoind_async_client::corepc_types::model::ListUnspentItem;

use crate::error::Error;

/// Creates a new `bitcoind` RPC client.
pub(crate) fn new_bitcoind_client(
    url: &str,
    rpc_cookie: Option<&PathBuf>,
    rpc_user: Option<&str>,
    rpc_pass: Option<&str>,
) -> Result<Client, Error> {
    Client::new(
        url,
        match (rpc_cookie, rpc_user, rpc_pass) {
            (None, None, None) => Auth::None,
            (Some(path), _, _) => Auth::CookieFile(path.clone()),
            (_, Some(user), Some(pass)) => Auth::UserPass(user.into(), pass.into()),
            (_, Some(_), None) => panic!("rpc auth: missing rpc_pass"),
            (_, None, Some(_)) => panic!("rpc auth: missing rpc_user"),
        },
    )
    .map_err(|_| Error::RpcClient)
}

/// Converts a sync-client `ListUnspentResultEntry` into the
/// `corepc_types::ListUnspentItem` shape that
/// [`strata_btcio::writer::build_chunked_envelope_txs`] accepts.
pub(crate) fn to_corepc_list_unspent_item(
    entry: ListUnspentResultEntry,
) -> Result<ListUnspentItem, Error> {
    let address = entry.address.ok_or(Error::BitcoinD)?;
    Ok(ListUnspentItem {
        txid: entry.txid,
        vout: entry.vout,
        address,
        label: entry.label.unwrap_or_default(),
        script_pubkey: entry.script_pub_key,
        amount: bitcoin::SignedAmount::from_sat(entry.amount.to_sat() as i64),
        confirmations: entry.confirmations,
        redeem_script: entry.redeem_script,
        spendable: entry.spendable,
        solvable: entry.solvable,
        descriptor: entry.descriptor,
        safe: entry.safe,
        parent_descriptors: None,
    })
}

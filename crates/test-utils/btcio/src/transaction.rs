use std::collections::HashMap;

use anyhow::Context;
use bitcoin::{OutPoint, Transaction, TxIn, TxOut, Txid, absolute::LockTime, transaction::Version};
use bitcoind_async_client::{Client, traits::Reader};
use corepc_node::Node;

/// Ensure a transaction has at least one output and uses the canonical version/locktime.
pub fn ensure_standard_transaction(tx: &mut Transaction) -> anyhow::Result<()> {
    if tx.output.is_empty() {
        anyhow::bail!("Transaction must have at least one output");
    }
    tx.version = Version::TWO;
    tx.lock_time = LockTime::ZERO;
    Ok(())
}

/// Collect the previous outputs for each transaction input.
///
/// `known_prevouts` can be used to seed the lookup (e.g. the funding input that was just created)
/// to avoid RPC round-trips.
pub async fn collect_prevouts(
    client: &Client,
    txins: &[TxIn],
    known_prevouts: &HashMap<OutPoint, TxOut>,
) -> anyhow::Result<Vec<TxOut>> {
    let mut prevouts = Vec::with_capacity(txins.len());
    for txin in txins {
        if let Some(prev) = known_prevouts.get(&txin.previous_output) {
            prevouts.push(prev.clone());
            continue;
        }

        let tx = client
            .get_raw_transaction_verbosity_zero(&txin.previous_output.txid)
            .await?
            .0;

        let prev_out = tx
            .output
            .get(txin.previous_output.vout as usize)
            .cloned()
            .with_context(|| format!("Prevout not found for {:?}", txin.previous_output))?;
        prevouts.push(prev_out);
    }
    Ok(prevouts)
}

/// Broadcast a transaction via the provided node and return the parsed txid.
///
/// A debug assertion ensures the returned txid matches the locally computed txid.
pub fn broadcast_transaction(bitcoind: &Node, tx: &Transaction) -> anyhow::Result<Txid> {
    let txid_wrapper = bitcoind.client.send_raw_transaction(tx)?;
    let txid_str = txid_wrapper.0.to_string();
    let txid: Txid = txid_str.parse()?;
    debug_assert_eq!(tx.compute_txid(), txid);
    Ok(txid)
}

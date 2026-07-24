//! Hydrates source-known DA transaction IDs from Bitcoin.

use std::iter::once;

use alloy_primitives::hex;
use anyhow::{bail, Context, Result};
use bitcoin::consensus::serialize;
use bitcoind_async_client::{traits::Reader, Auth, Client};

use super::reconstruct::ReplayManifest;

/// Fetches every source-known transaction from Bitcoin and verifies its txid.
pub(super) async fn hydrate_manifest(
    manifest: &mut ReplayManifest,
    rpc_url: &str,
    rpc_user: &str,
    rpc_password: &str,
) -> Result<()> {
    let client = Client::new(
        rpc_url.to_owned(),
        Auth::UserPass(rpc_user.to_owned(), rpc_password.to_owned()),
        None,
        None,
        None,
    )
    .context("creating Bitcoin RPC client")?;

    for batch in &manifest.batches {
        for expected_txid in once(&batch.commit_txid).chain(batch.reveal_txids.iter()) {
            if manifest.raw_transactions.contains_key(expected_txid) {
                continue;
            }
            let transaction = client
                .get_raw_transaction_verbosity_zero(expected_txid)
                .await
                .with_context(|| format!("fetching Bitcoin transaction {expected_txid}"))?
                .0;
            if transaction.compute_txid() != *expected_txid {
                bail!("Bitcoin returned the wrong transaction for {expected_txid}");
            }
            manifest
                .raw_transactions
                .insert(*expected_txid, hex::encode(serialize(&transaction)));
        }
    }

    Ok(())
}

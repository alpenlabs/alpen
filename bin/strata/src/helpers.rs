//! Helper utilities for the strata binary.

use std::time::Duration;

use anyhow::{Result, anyhow};
use bitcoin::Address;
use bitcoind_async_client::{Client, traits::Wallet};
use strata_btcio::BtcioParams;
use strata_params::Params;
use tokio::time;
use tracing::warn;

// Borrowed from old binary (bin/strata-client/src/main.rs).
// TODO: these might need to come from config.
const SEQ_ADDR_GENERATION_TIMEOUT: u64 = 10; // seconds
const BITCOIN_POLL_INTERVAL: u64 = 200; // millis

/// Gets an address controlled by sequencer's bitcoin wallet.
pub(crate) async fn generate_sequencer_address(bitcoin_client: &Client) -> Result<Address> {
    let mut last_err = None;
    time::timeout(Duration::from_secs(SEQ_ADDR_GENERATION_TIMEOUT), async {
        loop {
            match bitcoin_client.get_new_address().await {
                Ok(address) => return address,
                Err(err) => {
                    warn!(err = ?err, "failed to generate address");
                    last_err.replace(err);
                }
            }
            time::sleep(Duration::from_millis(BITCOIN_POLL_INTERVAL)).await;
        }
    })
    .await
    .map_err(|_| match last_err {
        None => anyhow!("failed to generate address; timeout"),
        Some(client_error) => {
            anyhow::Error::from(client_error).context("failed to generate address")
        }
    })
}

/// Converts [`Params`] to [`BtcioParams`] for use by btcio components.
pub(crate) fn params_to_btcio_params(params: &Params) -> BtcioParams {
    let rollup = params.rollup();
    BtcioParams::new(
        rollup.l1_reorg_safe_depth,
        rollup.magic_bytes,
        rollup.genesis_l1_view.height_u64(),
    )
}

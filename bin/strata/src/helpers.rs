//! Helper utilities for the strata binary.

#[cfg(feature = "sequencer")]
use std::time::Duration;

#[cfg(feature = "sequencer")]
use anyhow::{Result, anyhow};
#[cfg(feature = "sequencer")]
use bitcoin::Address;
#[cfg(feature = "sequencer")]
use bitcoind_async_client::{Client, traits::Wallet};
use strata_asm_params::AsmParams;
use strata_btcio::BtcioParams;
use strata_identifiers::Buf32;
use strata_predicate::PredicateTypeId;
#[cfg(feature = "sequencer")]
use tokio::time;
#[cfg(feature = "sequencer")]
use tracing::warn;

// Borrowed from old binary (bin/strata-client/src/main.rs).
// TODO(STR-3050): these might need to come from config.
#[cfg(feature = "sequencer")]
const SEQ_ADDR_GENERATION_TIMEOUT: u64 = 10; // seconds
#[cfg(feature = "sequencer")]
const BITCOIN_POLL_INTERVAL: u64 = 200; // millis

/// Gets an address controlled by sequencer's bitcoin wallet.
#[cfg(feature = "sequencer")]
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

/// Builds [`BtcioParams`] from the ASM params and the configured reorg-safe depth.
///
/// The magic bytes and genesis L1 height come from the ASM anchor; the reorg-safe
/// depth is an operational knob sourced from the node `[btcio]` config.
pub(crate) fn build_btcio_params(asm_params: &AsmParams, l1_reorg_safe_depth: u32) -> BtcioParams {
    BtcioParams::new(
        l1_reorg_safe_depth,
        asm_params.magic,
        asm_params.anchor.block.height(),
    )
}

/// Returns the sequencer's BIP340 schnorr key from the ASM checkpoint config's
/// sequencer predicate, when that predicate is a schnorr key.
///
/// Returns `None` for any other predicate type (or when no checkpoint subprotocol
/// is configured); the key is used only to decide whether checkpoint envelopes are
/// signed by an external signer and to report the sequencer pubkey over RPC.
pub(crate) fn sequencer_schnorr_key(asm_params: &AsmParams) -> Option<Buf32> {
    let predicate = &asm_params.checkpoint_config()?.sequencer_predicate;
    if predicate.id() == PredicateTypeId::Bip340Schnorr.as_u8() {
        Buf32::try_from(predicate.condition()).ok()
    } else {
        None
    }
}

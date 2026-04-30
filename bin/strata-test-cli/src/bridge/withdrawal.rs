//! Withdrawal fulfillment transaction functionality
//!
//! The CLI is responsible for wallet/UTXO management only.
//! All transaction structure and OP_RETURN construction is handled by
//! asm/subprotocols/bridge-v1/txs.

use anyhow::Context;
use bdk_wallet::{
    bitcoin::{consensus::serialize, Amount, FeeRate, ScriptBuf, Transaction},
    TxOrdering,
};
use strata_asm_proto_bridge_v1_txs::withdrawal_fulfillment::WithdrawalFulfillmentTxHeaderAux;
use strata_l1_txfmt::ParseConfig;
use strata_primitives::bitcoin_bosd::Descriptor;

use super::types::BitcoinDConfig;
use crate::{
    constants::MAGIC_BYTES,
    taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Creates a withdrawal fulfillment transaction (CLI wrapper)
///
/// Handles wallet operations (UTXO selection, signing) while using
/// asm/subprotocols/bridge-v1/txs for transaction structure.
///
/// # Arguments
/// * `recipient_bosd` - bosd specifying which address to send to
/// * `amount` - Amount to send in satoshis
/// * `deposit_idx` - Deposit index
/// * `bitcoind_config` - Bitcoind config
pub(crate) fn create_withdrawal_fulfillment_cli(
    recipient_bosd: String,
    amount: u64,
    deposit_idx: u32,
    bitcoind_config: BitcoinDConfig,
) -> anyhow::Result<Vec<u8>> {
    let recipient_script = recipient_bosd
        .parse::<Descriptor>()
        .map_err(|_| anyhow::anyhow!("not a valid bosd"))?
        .to_script();

    let tx = create_withdrawal_fulfillment_inner(
        recipient_script,
        amount,
        deposit_idx,
        bitcoind_config,
    )?;

    Ok(serialize(&tx))
}

/// Internal implementation of withdrawal fulfillment creation
fn create_withdrawal_fulfillment_inner(
    recipient_script: ScriptBuf,
    amount: u64,
    deposit_idx: u32,
    bitcoind_config: BitcoinDConfig,
) -> anyhow::Result<Transaction> {
    // Parse inputs
    let amount = Amount::from_sat(amount);

    // Create withdrawal fulfillment SPS50 tag
    let sps50_tag = WithdrawalFulfillmentTxHeaderAux::new(deposit_idx).build_tag_data();
    let sps_50_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&sps50_tag.as_ref())
        .context("failed to encode SPS-50 script")?;

    // Use wallet to select and fund inputs (CLI responsibility)
    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        &bitcoind_config.bitcoind_url,
        None,
        Some(&bitcoind_config.bitcoind_user),
        Some(&bitcoind_config.bitcoind_password),
    )?;

    sync_wallet(&mut wallet, &client)?;

    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    // Build PSBT using wallet for funding
    let mut psbt = {
        let mut builder = wallet.build_tx();

        builder.ordering(TxOrdering::Untouched);

        builder.add_recipient(sps_50_script, Amount::ZERO);
        builder.add_recipient(recipient_script.clone(), amount);

        builder.fee_rate(fee_rate);
        builder.finish().context("invalid PSBT")?
    };

    // Sign the PSBT
    wallet
        .sign(&mut psbt, Default::default())
        .context("signing failed")?;

    let tx = psbt.extract_tx().context("transaction extraction failed")?;

    Ok(tx)
}

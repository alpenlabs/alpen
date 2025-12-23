use bitcoin::{Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, Txid, Witness};
use bitcoind_async_client::{Client, traits::Reader};
use corepc_node::Node;

const DEFAULT_FEE_ESTIMATE_SATS: u64 = 1_000;

/// Adds a funding input to `tx` by creating a new UTXO to `address`.
///
/// This helper:
/// - Estimates the remaining value required (outputs minus existing non-null inputs) and adds a
///   conservative fee buffer.
/// - Calls into `bitcoind` to create a new UTXO paying to `address`.
/// - Inserts that outpoint into `tx`, replacing the first placeholder input if present or appending
///   otherwise.
///
/// Returns the index of the inserted input so the caller can update the witness/signature in place.
pub async fn add_funding_input(
    bitcoind: &Node,
    client: &Client,
    tx: &mut Transaction,
    address: &Address,
) -> anyhow::Result<usize> {
    // Estimate how much we need in total by summing outputs and subtracting any confirmed prevouts
    // that already provide value.
    let mut existing_value = 0u64;
    for input in &tx.input {
        if input.previous_output == OutPoint::null() {
            continue;
        }

        let prev_tx = client
            .get_raw_transaction_verbosity_zero(&input.previous_output.txid)
            .await?
            .0;

        if let Some(prev_out) = prev_tx.output.get(input.previous_output.vout as usize) {
            existing_value = existing_value.saturating_add(prev_out.value.to_sat());
        }
    }

    let desired_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
    let deficit = desired_value.saturating_sub(existing_value);
    let funding_amount = Amount::from_sat(deficit.saturating_add(DEFAULT_FEE_ESTIMATE_SATS.max(1)));

    // Create the funding output. This keeps the txid handy so we can reuse the prevout without
    // another RPC round trip.
    let (funding_txid, prev_vout) =
        create_funding_utxo(bitcoind, client, address, funding_amount).await?;
    let outpoint = OutPoint::new(funding_txid, prev_vout);

    // Insert the funding input, replacing the first placeholder if one exists.
    let txin = TxIn {
        previous_output: outpoint,
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    };

    for (idx, existing) in tx.input.iter_mut().enumerate() {
        if existing.previous_output == OutPoint::null() {
            *existing = txin;
            return Ok(idx);
        }
    }

    tx.input.push(txin);
    Ok(tx.input.len() - 1)
}

/// Create and confirm a funding UTXO locked to the given address.
async fn create_funding_utxo(
    bitcoind: &Node,
    client: &Client,
    address: &Address,
    amount: Amount,
) -> anyhow::Result<(Txid, u32)> {
    let funding_txid_str = bitcoind
        .client
        .send_to_address(address, amount)?
        .0
        .to_string();
    let funding_txid: Txid = funding_txid_str.parse()?;

    let funding_tx = client
        .get_raw_transaction_verbosity_zero(&funding_txid)
        .await?
        .0;

    let prev_vout = funding_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| output.script_pubkey == address.script_pubkey())
        .map(|(idx, _)| idx as u32)
        .ok_or_else(|| anyhow::anyhow!("Could not find output in funding transaction"))?;

    Ok((funding_txid, prev_vout))
}

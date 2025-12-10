use bitcoin::{Address, Amount, TxOut, Txid};
use bitcoind_async_client::{Client, traits::Reader};
use corepc_node::Node;

/// Create and confirm a funding UTXO locked to the given address.
///
/// # Returns
/// A tuple of (funding_txid, prev_vout, prev_output)
pub async fn create_funding_utxo(
    bitcoind: &Node,
    client: &Client,
    address: &Address,
    amount: Amount,
) -> anyhow::Result<(Txid, u32, TxOut)> {
    // Fund the address using the wallet
    let funding_txid_str = bitcoind
        .client
        .send_to_address(address, amount)?
        .0
        .to_string();
    let funding_txid: Txid = funding_txid_str.parse()?;

    // Get the funding transaction BEFORE mining (while it's still in mempool)
    let funding_tx_result = client
        .get_raw_transaction_verbosity_zero(&funding_txid)
        .await?;
    let funding_tx = funding_tx_result
        .transaction()
        .map_err(|e| anyhow::anyhow!("Failed to decode funding transaction: {}", e))?;

    // Find the output that pays to our address
    let (prev_vout, prev_output) = funding_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| output.script_pubkey == address.script_pubkey())
        .map(|(idx, output)| (idx as u32, output.clone()))
        .ok_or_else(|| anyhow::anyhow!("Could not find output in funding transaction"))?;

    Ok((funding_txid, prev_vout, prev_output))
}

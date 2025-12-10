use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    absolute::LockTime, transaction::Version,
};
use bitcoind_async_client::{Client, traits::Reader};
use corepc_node::Node;
use strata_crypto::EvenSecretKey;

use crate::{
    address::{derive_musig2_p2tr_address, derive_p2tr_address},
    funding::create_funding_utxo,
    signing::{sign_musig2_keypath, sign_taproot_transaction},
    utils::block_on,
};

/// Helper to sign and broadcast a transaction using a specific private key.
///
/// This function creates funding UTXOs locked to the P2TR address derived from the private key,
/// then creates a transaction spending from those UTXOs to the specified outputs.
///
/// The function automatically calculates the required funding amount based on the transaction
/// outputs and estimated fees, then adjusts the transaction to pay the appropriate fee.
///
/// # Arguments
/// * `bitcoind` - The bitcoind node
/// * `client` - The RPC client
/// * `secret_key` - The private key to sign with
/// * `mut tx` - The transaction to sign and broadcast (inputs will be added, fee will be
///   calculated)
///
/// # Returns
/// The txid of the signed and broadcast transaction
pub async fn submit_transaction_with_key(
    bitcoind: &Node,
    client: &Client,
    secret_key: &EvenSecretKey,
    mut tx: Transaction,
) -> anyhow::Result<Txid> {
    if tx.output.is_empty() {
        return Err(anyhow::anyhow!("Transaction must have at least one output"));
    }

    // Derive P2TR address from secret key
    let (p2tr_address, keypair, internal_key) = derive_p2tr_address(secret_key);

    // Calculate funding amount
    let total_output_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
    let estimated_fee = 1000u64; // Conservative estimate in sats
    let funding_amount = Amount::from_sat(total_output_value + estimated_fee);

    // Create and confirm funding UTXO
    let (funding_txid, prev_vout, prev_output) =
        create_funding_utxo(bitcoind, client, &p2tr_address, funding_amount).await?;

    // Add input to the transaction
    tx.input = vec![TxIn {
        previous_output: OutPoint::new(funding_txid, prev_vout),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    // Ensure transaction has proper version and locktime
    tx.version = Version::TWO;
    tx.lock_time = LockTime::ZERO;

    // Sign the transaction
    let signature = sign_taproot_transaction(&tx, &keypair, &internal_key, &prev_output, 0)?;

    // Add the signature to the witness (Taproot key-spend signatures are 64 bytes, no sighash type
    // appended for Default)
    tx.input[0].witness.push(signature.as_ref());

    // Broadcast the transaction
    let txid_wrapper = bitcoind.client.send_raw_transaction(&tx)?;
    let core_txid = txid_wrapper.0;
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    Ok(txid)
}

/// Helper to sign and broadcast a transaction using multiple secret keys with MuSig2 aggregation.
///
/// This function creates funding UTXOs locked to a P2TR address derived from MuSig2 aggregated
/// public keys, then creates a transaction spending from those UTXOs to the specified outputs
/// using MuSig2 signature aggregation.
///
/// # Arguments
/// * `bitcoind` - The bitcoind node
/// * `client` - The RPC client
/// * `secret_keys` - Slice of secret keys to aggregate for signing
/// * `mut tx` - The transaction to sign and broadcast (inputs will be added, fee will be
///   calculated)
///
/// # Returns
/// The txid of the signed and broadcast transaction
pub async fn submit_transaction_with_keys(
    bitcoind: &Node,
    client: &Client,
    secret_keys: &[EvenSecretKey],
    tx: &mut Transaction,
) -> anyhow::Result<Txid> {
    if tx.output.is_empty() {
        return Err(anyhow::anyhow!("Transaction must have at least one output"));
    }

    if secret_keys.is_empty() {
        return Err(anyhow::anyhow!("At least one secret key is required"));
    }

    // Derive MuSig2 aggregated P2TR address
    let (p2tr_address, aggregated_internal_key) = derive_musig2_p2tr_address(secret_keys)?;

    // Calculate funding amount
    let total_output_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
    let estimated_fee = 1000u64; // Conservative estimate in sats
    let funding_amount = Amount::from_sat(total_output_value + estimated_fee);

    // Create and confirm funding UTXO
    let (funding_txid, prev_vout, funding_prev_output) =
        create_funding_utxo(bitcoind, client, &p2tr_address, funding_amount).await?;

    // Replace a null outpoint with the funding input; if none exist, append at the end to preserve
    // the ordering of existing inputs.
    let funding_input = TxIn {
        previous_output: OutPoint::new(funding_txid, prev_vout),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    };

    if let Some((idx, _)) = tx
        .input
        .iter()
        .enumerate()
        .find(|(_, inp)| inp.previous_output == OutPoint::null())
    {
        tx.input[idx] = funding_input;
    } else {
        tx.input.push(funding_input);
    }

    // Bail out if any null outpoints remain; we cannot sign or broadcast them.
    if let Some(idx) = tx
        .input
        .iter()
        .position(|inp| inp.previous_output == OutPoint::null())
    {
        return Err(anyhow::anyhow!(
            "Cannot submit transaction: input {} still has a null outpoint",
            idx
        ));
    }

    // Ensure transaction has proper version and locktime
    tx.version = Version::TWO;
    tx.lock_time = LockTime::ZERO;

    // Collect prevouts in input order so signatures can be produced without reordering txins.
    let mut prevouts: Vec<TxOut> = Vec::with_capacity(tx.input.len());
    for txin in &tx.input {
        if txin.previous_output.txid == funding_txid && txin.previous_output.vout == prev_vout {
            prevouts.push(funding_prev_output.clone());
        } else {
            // Fetch the prevout from the node to sign the existing input.
            let raw_tx = client
                .get_raw_transaction_verbosity_zero(&txin.previous_output.txid)
                .await?
                .transaction()
                .map_err(|e| anyhow::anyhow!("Failed to decode prev transaction: {}", e))?;

            let prev_out = raw_tx
                .output
                .get(txin.previous_output.vout as usize)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!("Prevout not found for {:?}", txin.previous_output)
                })?;
            prevouts.push(prev_out);
        }
    }

    // Sign each input in place using the aggregated MuSig2 key.
    for idx in 0..tx.input.len() {
        // Skip inputs that already carry a witness (e.g., pre-signed script path spends).
        if !tx.input[idx].witness.is_empty() {
            continue;
        }

        let sig = sign_musig2_keypath(tx, secret_keys, &prevouts, idx)?;
        tx.input[idx].witness.push(sig.as_ref());
    }

    // Broadcast the transaction
    let txid_wrapper = bitcoind.client.send_raw_transaction(tx)?;
    let core_txid = txid_wrapper.0;
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    Ok(txid)
}

pub fn submit_transaction_with_keys_blocking(
    bitcoind: &Node,
    client: &Client,
    secret_keys: &[EvenSecretKey],
    tx: &mut Transaction,
) -> anyhow::Result<Txid> {
    block_on(submit_transaction_with_keys(
        bitcoind,
        client,
        secret_keys,
        tx,
    ))
}

#[cfg(test)]
mod tests {
    use bitcoin::{BlockHash, TxOut, secp256k1::Secp256k1};
    use bitcoind_async_client::traits::{Reader, Wallet};

    use super::*;
    use crate::{client::get_bitcoind_and_client, mining::mine_blocks};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_submit_transaction_with_key() {
        // Setup
        let (node, client) = get_bitcoind_and_client();

        // Mine some blocks to fund the wallet (need 101+ for coinbase maturity)
        let _ = mine_blocks(&node, &client, 101, None).await.unwrap();

        // Generate a random keypair
        let secp = Secp256k1::new();
        let (secret_key, _public_key) = secp.generate_keypair(&mut rand::thread_rng());
        let even_secret_key: EvenSecretKey = secret_key.into();

        // Create the transaction with desired outputs
        let output_amount = Amount::from_sat(50_000);
        let recipient_address = client.get_new_address().await.unwrap();
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![], // Will be populated by submit_transaction_with_key
            output: vec![TxOut {
                value: output_amount,
                script_pubkey: recipient_address.script_pubkey(),
            }],
        };

        // Submit the transaction using the new function
        let txid = submit_transaction_with_key(&node, &client, &even_secret_key, tx)
            .await
            .unwrap();
        println!("Transaction submitted with txid: {}", txid);
        _ = mine_blocks(&node, &client, 1, None).await;

        // Verify the transaction is confirmed
        let blockchain_info = client.get_blockchain_info().await.unwrap();
        let block_hash: BlockHash = blockchain_info.best_block_hash.parse().unwrap();
        let block = client.get_block(&block_hash).await.unwrap();

        // Check that our transaction is in the block
        let tx_found = block.txdata.iter().any(|tx| tx.compute_txid() == txid);

        assert!(
            tx_found,
            "Transaction {} should be included in block {}",
            txid, block_hash
        );

        println!("✓ Transaction confirmed in block {}", block_hash);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_submit_transaction_with_keys_musig2() {
        // Setup
        let (node, client) = get_bitcoind_and_client();

        // Mine some blocks to fund the wallet (need 101+ for coinbase maturity)
        let _ = mine_blocks(&node, &client, 101, None).await.unwrap();

        // Generate multiple random keypairs for MuSig2
        let secp = Secp256k1::new();
        let num_signers = 3;
        let secret_keys: Vec<EvenSecretKey> = (0..num_signers)
            .map(|_| {
                let (sk, _pk) = secp.generate_keypair(&mut rand::thread_rng());
                EvenSecretKey::from(sk)
            })
            .collect();

        println!("Created {} secret keys for MuSig2 aggregation", num_signers);

        // Create the transaction with desired outputs
        let output_amount = Amount::from_sat(75_000);
        let recipient_address = client.get_new_address().await.unwrap();
        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![], // Will be populated by submit_transaction_with_keys
            output: vec![TxOut {
                value: output_amount,
                script_pubkey: recipient_address.script_pubkey(),
            }],
        };

        // Submit the transaction using MuSig2 aggregation
        let txid = submit_transaction_with_keys(&node, &client, &secret_keys, &mut tx)
            .await
            .unwrap();
        println!("MuSig2 transaction submitted with txid: {}", txid);
        _ = mine_blocks(&node, &client, 1, None).await;

        // Verify the transaction is confirmed
        let blockchain_info = client.get_blockchain_info().await.unwrap();
        let block_hash: BlockHash = blockchain_info.best_block_hash.parse().unwrap();
        let block = client.get_block(&block_hash).await.unwrap();

        // Check that our transaction is in the block
        let tx_found = block.txdata.iter().any(|tx| tx.compute_txid() == txid);

        assert!(
            tx_found,
            "MuSig2 transaction {} should be included in block {}",
            txid, block_hash
        );

        println!("✓ MuSig2 transaction confirmed in block {}", block_hash);

        // Verify the transaction has the correct witness structure
        let confirmed_tx = block
            .txdata
            .iter()
            .find(|tx| tx.compute_txid() == txid)
            .unwrap();

        assert_eq!(
            confirmed_tx.input.len(),
            1,
            "Transaction should have exactly 1 input"
        );

        let witness = &confirmed_tx.input[0].witness;
        assert_eq!(
            witness.len(),
            1,
            "Taproot key-spend witness should have exactly 1 element (the signature)"
        );
    }
}

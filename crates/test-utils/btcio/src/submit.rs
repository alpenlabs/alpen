use std::collections::HashMap;

use bitcoin::{OutPoint, Transaction, Txid};
use bitcoind_async_client::Client;
use corepc_node::Node;
use strata_crypto::{EvenSecretKey, test_utils::schnorr::Musig2Tweak};

use crate::{
    address::{derive_musig2_p2tr_address, derive_p2tr_address},
    funding::add_funding_input,
    signing::{sign_musig2_keypath, sign_taproot_transaction},
    transaction::{broadcast_transaction, collect_prevouts, ensure_standard_transaction},
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
    tx: &mut Transaction,
) -> anyhow::Result<Txid> {
    ensure_standard_transaction(tx)?;

    // Derive P2TR address from secret key
    let (p2tr_address, keypair, internal_key) = derive_p2tr_address(secret_key);

    let funding_index = add_funding_input(bitcoind, client, tx, &p2tr_address).await?;
    let prevouts = collect_prevouts(client, &tx.input, &HashMap::new()).await?;

    // Sign the transaction
    let signature = sign_taproot_transaction(
        tx,
        &keypair,
        &internal_key,
        &prevouts[funding_index],
        funding_index,
    )?;

    // Add the signature to the witness (Taproot key-spend signatures are 64 bytes, no sighash type
    // appended for Default)
    tx.input[funding_index].witness.push(signature.as_ref());

    broadcast_transaction(bitcoind, tx)
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
    input_tweaks: Option<&HashMap<OutPoint, Musig2Tweak>>,
) -> anyhow::Result<Txid> {
    if secret_keys.is_empty() {
        return Err(anyhow::anyhow!("At least one secret key is required"));
    }
    ensure_standard_transaction(tx)?;

    // Derive MuSig2 aggregated P2TR address
    let (p2tr_address, _aggregated_internal_key) = derive_musig2_p2tr_address(secret_keys)?;

    let _funding_index = add_funding_input(bitcoind, client, tx, &p2tr_address).await?;
    let prevouts = collect_prevouts(client, &tx.input, &HashMap::new()).await?;

    // Sign each input in place using the aggregated MuSig2 key.
    for idx in 0..tx.input.len() {
        // Skip inputs that already carry a witness (e.g., pre-signed script path spends).
        if !tx.input[idx].witness.is_empty() {
            continue;
        }

        let tweak = input_tweaks
            .and_then(|map| map.get(&tx.input[idx].previous_output))
            .copied()
            .unwrap_or(Musig2Tweak::TaprootKeySpend);
        let sig = sign_musig2_keypath(tx, secret_keys, &prevouts, idx, tweak)?;
        tx.input[idx].witness.push(sig.as_ref());
    }

    broadcast_transaction(bitcoind, tx)
}

#[cfg(test)]
mod tests {
    use bitcoin::{Amount, BlockHash, TxOut, absolute::LockTime, transaction::Version};
    use bitcoind_async_client::traits::{Reader, Wallet};
    use musig2::secp256k1::SECP256K1;

    use super::*;
    use crate::{client::get_bitcoind_and_client, mining::mine_blocks};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_submit_transaction_with_key() {
        // Setup
        let (node, client) = get_bitcoind_and_client();

        // Mine some blocks to fund the wallet (need 101+ for coinbase maturity)
        let _ = mine_blocks(&node, &client, 101, None).await.unwrap();

        // Generate a random keypair
        let (secret_key, _public_key) = SECP256K1.generate_keypair(&mut rand::thread_rng());
        let even_secret_key: EvenSecretKey = secret_key.into();

        // Create the transaction with desired outputs
        let output_amount = Amount::from_sat(50_000);
        let recipient_address = client.get_new_address().await.unwrap();
        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![], // Will be populated by submit_transaction_with_key
            output: vec![TxOut {
                value: output_amount,
                script_pubkey: recipient_address.script_pubkey(),
            }],
        };

        // Submit the transaction using the new function
        let txid = submit_transaction_with_key(&node, &client, &even_secret_key, &mut tx)
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
        let num_signers = 3;
        let secret_keys: Vec<EvenSecretKey> = (0..num_signers)
            .map(|_| {
                let (sk, _pk) = SECP256K1.generate_keypair(&mut rand::thread_rng());
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
        let txid = submit_transaction_with_keys(&node, &client, &secret_keys, &mut tx, None)
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

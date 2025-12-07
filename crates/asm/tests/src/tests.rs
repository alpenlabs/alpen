use bitcoin::{secp256k1::Secp256k1, Amount};
use bitcoind_async_client::traits::{Reader, Wallet};
use strata_crypto::EvenSecretKey;

use crate::{
    setup::setup_env,
    utils::{get_bitcoind_and_client, mine_blocks, submit_transaction_with_key},
};

#[tokio::test(flavor = "multi_thread")]
async fn test_asm_transition() {
    // 1. Setup Environment
    let env = setup_env().await;
    let client = env.client;
    let node = env._node;
    let service_state = env.service_state;

    // 2. Create a new block to test transition
    // We mine 1 block on top of tip (which is our genesis).
    let address = client.get_new_address().await.unwrap();
    let new_block_hashes = mine_blocks(&node, &client, 1, Some(address)).await.unwrap();
    let new_block_hash = new_block_hashes[0];

    let new_block = client.get_block(&new_block_hash).await.unwrap();

    println!("Mined new block: {}", new_block_hash);

    // 6. Call Transition
    // The transition function expects the block to be a child of the current anchor.
    // Current anchor is at 101. New block is at 102, parent is 101.
    // This should work.

    let result = service_state.transition(&new_block);

    match result {
        Ok(_output) => {
            println!("Transition successful!");
            // Verify output if needed.
            // Since block is empty (coinbase only), `compute_asm_transition` should return a
            // state that reflects an empty transition or just L1 updates.
            // We mainly care that it didn't error.
        }
        Err(e) => {
            panic!("Transition failed: {:?}", e);
        }
    }
}

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
    let tx = bitcoin::Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![], // Will be populated by submit_transaction_with_key
        output: vec![bitcoin::TxOut {
            value: output_amount,
            script_pubkey: recipient_address.script_pubkey(),
        }],
    };

    println!(
        "Creating transaction with {} sat output",
        output_amount.to_sat()
    );

    // Submit the transaction using the new function
    let result = submit_transaction_with_key(&node, &client, &even_secret_key, tx).await;
    assert!(
        result.is_ok(),
        "Failed to submit transaction: {:?}",
        result.err()
    );

    let txid = result.unwrap();
    println!("Transaction submitted with txid: {}", txid);

    // Verify the transaction is confirmed
    let blockchain_info = client.get_blockchain_info().await.unwrap();
    let block_hash: bitcoin::BlockHash = blockchain_info.best_block_hash.parse().unwrap();
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

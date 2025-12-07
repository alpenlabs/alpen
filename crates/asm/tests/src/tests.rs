use bitcoin::{
    absolute::LockTime, secp256k1::Secp256k1, transaction::Version, Amount, BlockHash, Transaction,
    TxOut,
};
use bitcoind_async_client::traits::{Reader, Wallet};
use strata_crypto::EvenSecretKey;

use crate::{
    setup::setup_env,
    utils::{
        get_bitcoind_and_client, mine_blocks, submit_transaction_with_key,
        submit_transaction_with_keys,
    },
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
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![], // Will be populated by submit_transaction_with_keys
        output: vec![TxOut {
            value: output_amount,
            script_pubkey: recipient_address.script_pubkey(),
        }],
    };

    // Submit the transaction using MuSig2 aggregation
    let txid = submit_transaction_with_keys(&node, &client, &secret_keys, tx)
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

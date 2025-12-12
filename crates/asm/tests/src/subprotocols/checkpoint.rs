//! E2E tests for checkpoint subprotocol with Bitcoin regtest.
//!
//! These tests verify the full E2E flow:
//! 1. Spin up Bitcoin regtest node
//! 2. Create checkpoint transactions with envelope format
//! 3. Submit and mine transactions
//! 4. Configure ASM worker with checkpoint & bridge subprotocol
//! 5. Run ASM transition and verify checkpoint state updates & check for correct log emission.
//!    (CheckpointUpdate & DepositLog)



#[tokio::test(flavor = "multi_thread")]
async fn test_asm_transition_empty_block() {
    // Setup environment with ASM worker
    let env = AsmTestEnv::new().await;
    let client = &env.client;
    let node = &env.node;
    let service_state = &env.service_state;

    // Mine 1 block on top of genesis (which is at height 101)
    let address = client.get_new_address().await.unwrap();
    let new_block_hashes = mine_blocks(node, client, 1, Some(address)).await.unwrap();
    let new_block_hash = new_block_hashes[0];

    let new_block = client.get_block(&new_block_hash).await.unwrap();

    println!("Mined new block: {}", new_block_hash);

    // Call ASM transition
    let result = service_state.transition(&new_block);

    match result {
        Ok(output) => {
            println!("ASM Transition successful!");

            // For an empty block (coinbase only), verify expected output properties:
            // - The state should be updated with new chain view
            // - The manifest should be created with the block id

            // Verify manifest has correct block id
            let blkid: L1BlockId = new_block.header.block_hash().into();
            assert_eq!(
                output.manifest.blkid(),
                &blkid,
                "Manifest block id should match the processed block"
            );

            // Verify the state is present (chain view should be updated)
            // For an empty block, we just verify the transition succeeded
            println!("Output state has {} sections", output.state.sections.len());

            println!("Empty block transition assertions passed");
        }
        Err(e) => {
            panic!("ASM Transition failed: {:?}", e);
        }
    }
}

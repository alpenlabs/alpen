#[cfg(test)]
mod context;
#[cfg(test)]
mod setup;
#[cfg(test)]
mod utils;

#[cfg(test)]
mod tests {
    use bitcoind_async_client::traits::{Reader, Wallet};

    use crate::{setup::setup_env, utils::mine_blocks};

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
}

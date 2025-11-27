import flexitest

from envs import testenv


@flexitest.register
class EeConsistencyRecoveryTest(testenv.StrataTestBase):
    """
    Tests that the sequencer can recover missing blocks in reth on startup.

    The scenario simulates reth losing recent blocks due to consistency issues
    during forced exits. The sequencer should detect this on startup and replay
    the missing blocks using stored exec payloads.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(110))

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()

        seq_waiter = self.create_strata_waiter(seqrpc)
        reth_waiter = self.create_reth_waiter(rethrpc)

        # Wait for genesis
        seq_waiter.wait_until_genesis()
        self.info("Genesis occurred, waiting for initial blocks...")

        # Wait for some blocks to be produced
        initial_blocks = 5
        seq_waiter.wait_until_chain_tip_exceeds(initial_blocks)
        self.info(f"Chain tip exceeded {initial_blocks} blocks")

        # Get current state before snapshot
        initial_eth_block = reth_waiter.get_current_block_number()
        self.info(f"Initial ETH block number: {initial_eth_block}")

        # Take a snapshot of reth's data directory
        # This simulates the state before some blocks are lost
        reth.stop()
        self.info("Stopped reth, taking snapshot...")
        reth.snapshot_datadir(1)

        # Restart reth to continue producing blocks
        reth.start()
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc)
        reth_waiter.wait_until_eth_block_exceeds(0)
        self.info("Reth restarted and ready")

        # Produce more blocks (these will be "lost" when we restore the snapshot)
        additional_blocks = 5
        target_blocks = initial_blocks + additional_blocks
        seq_waiter.wait_until_chain_tip_exceeds(target_blocks)

        # Get the state after producing more blocks
        final_eth_block = reth_waiter.get_current_block_number()
        sync_status = seqrpc.strata_syncStatus()
        final_tip_slot = sync_status["tip_height"]
        self.info(f"Final ETH block: {final_eth_block}, Final tip slot: {final_tip_slot}")

        # Verify we have more blocks than before
        assert final_eth_block > initial_eth_block, (
            f"Expected more blocks than initial ({initial_eth_block}), got {final_eth_block}"
        )

        # Now simulate reth losing blocks by:
        # 1. Stop both sequencer and reth
        # 2. Restore reth to the earlier snapshot
        # 3. Restart both services
        #
        # The sequencer should detect the missing blocks in reth and recover them.

        self.info("Simulating reth data loss...")
        seq_signer = ctx.get_service("sequencer_signer")

        # Stop services
        seq_signer.stop()
        seq.stop()
        reth.stop()
        self.info("All services stopped")

        # Restore reth to the earlier snapshot (simulating data loss)
        reth.restore_snapshot(1)
        self.info("Reth snapshot restored (simulating data loss)")

        # Restart reth first
        reth.start()
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc)
        reth_waiter.wait_until_eth_block_exceeds(0)
        self.info("Reth restarted from older snapshot and ready")

        # Verify reth is at the older state (blocks were "lost")
        restored_eth_block = reth_waiter.get_current_block_number()
        self.info(f"Reth block after restore: {restored_eth_block}")

        # The restored block should be less than or equal to the initial block
        # (some blocks may have been produced between snapshot and getting block number)
        assert restored_eth_block <= initial_eth_block + 2, (
            f"Expected reth to be at or near initial state ({initial_eth_block}), "
            f"but got {restored_eth_block}"
        )

        # Now restart the sequencer - it should detect and recover missing blocks
        seq.start()
        seqrpc = seq.create_rpc()
        seq_waiter = self.create_strata_waiter(seqrpc)
        self.info("Sequencer restarted, checking for recovery...")

        # Wait for sequencer to be ready
        seq_waiter.wait_until_client_ready()
        self.info("Sequencer is ready")

        # Restart the signer
        seq_signer.start()

        # The sequencer's EE consistency check should have recovered the missing blocks
        # Wait for reth to catch up to the expected block height
        reth_waiter = self.create_reth_waiter(rethrpc, timeout=30)

        # Wait for reth to recover to (at least close to) the final block height
        # The recovery should have submitted the missing payloads to reth
        try:
            reth_waiter.wait_until_eth_block_at_least(
                final_eth_block - 1,  # Allow for some timing variance
                message=f"Waiting for reth to recover to block {final_eth_block - 1}",
            )
            recovered_eth_block = reth_waiter.get_current_block_number()
            self.info(f"Reth recovered to block: {recovered_eth_block}")
        except AssertionError:
            # Get current state for debugging
            current_eth_block = reth_waiter.get_current_block_number()
            self.error(
                f"Recovery may have failed. Current eth block: {current_eth_block}, "
                f"expected at least: {final_eth_block - 1}"
            )
            raise

        # Verify the chain can continue producing new blocks after recovery
        new_target = final_tip_slot + 2
        seq_waiter.wait_until_chain_tip_exceeds(new_target, timeout=30)
        self.info(f"Chain continued producing blocks past slot {new_target}")

        # Final verification - check reth is still synced
        final_check_eth_block = reth_waiter.get_current_block_number()
        assert final_check_eth_block >= final_eth_block, (
            f"Expected reth to be at or past {final_eth_block}, got {final_check_eth_block}"
        )

        self.info("EE consistency recovery test passed!")
        return True

import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import FullnodeDbtoolMixin
from utils.dbtool import send_tx


@flexitest.register
class RevertChainstateFnTest(FullnodeDbtoolMixin):
    """Test revert chainstate on fullnode"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.HubNetworkEnvConfig(
                110,
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Wait for genesis and generate some initial blocks
        seq_waiter = self.create_strata_waiter(self.seqrpc)
        seq_waiter.wait_until_genesis()

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(self.web3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        seq_waiter.wait_until_epoch_finalized(1, timeout=30)

        # Generate more blocks to have a longer chain beyond the finalized epoch
        for _ in range(10):
            send_tx(self.web3)

        # Wait for both services to be in sync
        old_seq_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        old_seq_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(
            f"Sequencer OL block number: {old_seq_ol_block_number}, "
            f"EL block number: {old_seq_el_block_number}"
        )

        # Check if both services are at the same state before proceeding
        if old_seq_ol_block_number != old_seq_el_block_number:
            self.warning(
                f"Sequencer OL and EL are not in sync: OL={old_seq_ol_block_number}, "
                f"EL={old_seq_el_block_number}"
            )

        old_fn_ol_block_number = self.follower_1_rpc.strata_syncStatus()["tip_height"]
        old_fn_el_block_number = int(self.follower_1_reth_rpc.eth_blockNumber(), base=16)
        self.info(
            f"Fullnode OL block number: {old_fn_ol_block_number}, "
            f"EL block number: {old_fn_el_block_number}"
        )

        # Check if both services are at the same state before proceeding
        if old_fn_ol_block_number != old_fn_el_block_number:
            self.warning(
                f"Fullnode OL and EL are not in sync: OL={old_fn_ol_block_number}, "
                f"EL={old_fn_el_block_number}"
            )

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()
        self.follower_1_node.stop()
        self.follower_1_reth.stop()

        # Get checkpoints summary to find the latest checkpoint
        self.info("Getting checkpoints summary to find latest checkpoint")
        checkpoints_summary = self.get_checkpoints_summary()

        checkpoints_count = checkpoints_summary.get("checkpoints_found_in_db", 0)
        if checkpoints_count == 0:
            self.error("No checkpoints found")
            return False

        # Get the latest checkpoint index (checkpoints_count - 1)
        checkpt_idx_before_revert = checkpoints_count - 1
        self.info(f"Latest checkpoint index: {checkpt_idx_before_revert}")

        # Get the latest checkpoint details
        checkpt_before_revert = self.get_checkpoint(checkpt_idx_before_revert).get("checkpoint", {})

        # Extract the L2 range from the checkpoint
        batch_info = checkpt_before_revert.get("commitment", {}).get("batch_info", {})
        l2_range = batch_info.get("l2_range", {})

        if not l2_range:
            self.error("Could not find L2 range in checkpoint")
            return False

        # Get the checkpoint end slot (last L2 block of the checkpointed range)
        checkpt_end_slot = l2_range[1].get("slot")
        checkpt_end_block_id = l2_range[1].get("blkid")

        if checkpt_end_slot is None:
            self.error("Could not find checkpoint end slot")
            return False

        self.info(f"Checkpoint end slot: {checkpt_end_slot}")
        self.info(f"Checkpoint end block ID: {checkpt_end_block_id}")

        # Get sync information to find the current tip
        sync_info = self.get_syncinfo()
        tip_block_id = sync_info.get("l2_tip_block_id")
        tip_slot = sync_info.get("l2_tip_height")

        if tip_slot is None or not tip_block_id:
            self.error("Could not find tip block information")
            return False

        self.info(f"Tip slot: {tip_slot}, tip block ID: {tip_block_id}")

        # Ensure we have blocks outside the checkpointed range
        if tip_slot <= checkpt_end_slot:
            self.info("No blocks outside checkpointed range - test cannot proceed")
            return True

        # Use the checkpoint end block as target (last L2 block of checkpointed range)
        target_block_id = checkpt_end_block_id
        target_slot = checkpt_end_slot

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Revert chainstate to the last L2 block of the checkpointed range
        # Using the fullnode (fn) database (follower_1_node)
        self.info(f"Testing revert-chainstate to {target_block_id} using fullnode (fn) database")

        return_code, stdout, stderr = self.revert_chainstate(target_block_id)

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Revert chainstate completed successfully")
        self.info(f"Stdout: {stdout}")

        # Verify chainstate was reverted correctly
        self.info("Verifying chainstate after revert")
        reverted_chainstate = self.get_chainstate(target_block_id)

        reverted_current_slot = reverted_chainstate.get("current_slot", 0)
        reverted_current_epoch = reverted_chainstate.get("current_epoch", 0)

        self.info(
            f"Reverted chainstate - current_slot: {reverted_current_slot}, "
            f"current_epoch: {reverted_current_epoch}"
        )

        # Verify that the chainstate was reverted to the target slot
        if reverted_current_slot != target_slot:
            self.error(
                f"Chainstate current_slot should be {target_slot} after revert, "
                f"got {reverted_current_slot}"
            )
            return False

        self.info("Chainstate revert verification passed")

        # Start services and verify they can continue from the reverted block
        self.reth.start()
        self.seq.start()
        self.seq_signer.start()
        self.follower_1_reth.start()
        self.follower_1_node.start()

        # Wait for block production to resume to sync
        seq_waiter.wait_until_chain_tip_exceeds(old_seq_ol_block_number + 1, timeout=30)

        # Wait for full node to catch up to sequencer
        fn_waiter = self.create_strata_waiter(self.follower_1_rpc)
        fn_waiter.wait_until_chain_tip_exceeds(old_seq_ol_block_number + 1, timeout=60)

        # Wait for new epoch summary to be created
        self.info("Waiting for new epoch summary to be created after restart")
        epoch_summary = fn_waiter.wait_until_chain_epoch(checkpt_idx_before_revert + 1, timeout=120)
        self.info(f"Epoch summary: {epoch_summary}")

        # Get final block numbers for verification
        seq_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        seq_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        fn_ol_block_number = self.follower_1_rpc.strata_syncStatus()["tip_height"]
        fn_el_block_number = int(self.follower_1_reth_rpc.eth_blockNumber(), base=16)

        self.info(f"After restart - Sequencer OL: {seq_ol_block_number}, EL: {seq_el_block_number}")
        self.info(f"After restart - Fullnode OL: {fn_ol_block_number}, EL: {fn_el_block_number}")

        # Check sequencer services sync status (warning only)
        if seq_ol_block_number != seq_el_block_number:
            self.warning(
                f"Sequencer services not in sync after restart: OL={seq_ol_block_number}, "
                f"EL={seq_el_block_number}"
            )

        # Check fullnode (fn) services sync status (warning only)
        if fn_ol_block_number != fn_el_block_number:
            self.warning(
                f"Fullnode (fn) services not in sync after restart: OL={fn_ol_block_number}, "
                f"EL={fn_el_block_number}"
            )

        # Verify fullnode (fn) has caught up to sequencer
        if fn_ol_block_number < seq_ol_block_number - 5:  # Allow some tolerance
            self.error(
                f"Fullnode (fn) did not catch up to sequencer: FN={fn_ol_block_number}, "
                f"SEQ={seq_ol_block_number}"
            )
            return False

        self.info("Successfully reverted full node chainstate and verified resync")
        return True

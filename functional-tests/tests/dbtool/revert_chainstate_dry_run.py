import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import SequencerDbtoolMixin
from utils.dbtool import get_latest_checkpoint, setup_revert_chainstate_test, target_start_of_epoch
from utils.utils import ProverClientSettings


@flexitest.register
class RevertChainstateDryRunTest(SequencerDbtoolMixin):
    """Test that revert-chainstate runs in dry run mode by default (without -f flag)"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Setup: wait for genesis, create transactions, finalize epoch
        setup_revert_chainstate_test(self)

        # Wait for both services to be in sync
        old_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        old_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {old_ol_block_number}, EL block number: {old_el_block_number}")

        # Check if both services are at the same state before proceeding
        if old_ol_block_number != old_el_block_number:
            self.warning(
                f"OL and EL are not in sync: OL={old_ol_block_number}, EL={old_el_block_number}"
            )

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get the latest checkpoint using helper
        checkpoint_info = get_latest_checkpoint(self)
        if checkpoint_info is None:
            return False

        latest_checkpt_idx = checkpoint_info["idx"]
        l2_range = checkpoint_info["l2_range"]

        # Use the checkpoint start as the target for dry run tests
        target_block_id, target_slot = target_start_of_epoch(l2_range)

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Get sync information to find the current tip
        sync_info = self.get_syncinfo()
        tip_block_id = sync_info.get("l2_tip_block_id")
        tip_slot = sync_info.get("l2_tip_height")

        if tip_slot is None or not tip_block_id:
            self.error("Could not find tip block information")
            return False

        self.info(f"Tip slot: {tip_slot}, tip block ID: {tip_block_id}")

        # Get chainstate before dry run
        self.info("Getting chainstate before dry run")
        chainstate_before = self.get_chainstate(tip_block_id)
        current_slot_before = chainstate_before.get("current_slot", 0)
        current_epoch_before = chainstate_before.get("current_epoch", 0)

        self.info(
            f"Chainstate before dry run - current_slot: {current_slot_before}, "
            f"current_epoch: {current_epoch_before}"
        )

        # Get database counts before dry run
        self.info("Getting database counts before dry run")

        # L2 blocks count
        l2_summary_before = self.get_l2_summary()
        l2_blocks_count_before = l2_summary_before.get("l2_blocks_in_db", 0)

        # Checkpoints count
        checkpoints_summary_before = self.get_checkpoints_summary()
        checkpoints_count_before = checkpoints_summary_before.get("checkpoints_found_in_db", 0)

        self.info("Before dry run:")
        self.info(f"  - L2 blocks: {l2_blocks_count_before}")
        self.info(f"  - Checkpoints: {checkpoints_count_before}")

        # Run dry run with -c and -d flags (without -f)
        self.info("=== Dry run test with -c and -d flags ===")
        self.info("Testing dry run of revert-chainstate with -c -d flags (without -f)")
        self.info("This should show what would be deleted but not actually delete anything")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-c", "-d")

        if return_code != 0:
            self.error(f"revert-chainstate dry run failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Dry run completed successfully")
        self.info(f"Stdout: {stdout}")

        # Verify database counts haven't changed
        self.info("\nVerifying database counts after dry run")

        # Get counts after dry run
        l2_summary_after = self.get_l2_summary()
        l2_blocks_count_after = l2_summary_after.get("l2_blocks_in_db", 0)

        checkpoints_summary_after = self.get_checkpoints_summary()
        checkpoints_count_after = checkpoints_summary_after.get("checkpoints_found_in_db", 0)

        self.info("After dry run:")
        self.info(f"  - L2 blocks: {l2_blocks_count_after}")
        self.info(f"  - Checkpoints: {checkpoints_count_after}")

        # Verify L2 blocks count unchanged
        if l2_blocks_count_before != l2_blocks_count_after:
            self.error(
                f"L2 blocks count changed during dry run! "
                f"Before: {l2_blocks_count_before}, After: {l2_blocks_count_after}"
            )
            return False

        # Verify checkpoints count unchanged
        if checkpoints_count_before != checkpoints_count_after:
            self.error(
                f"Checkpoints count changed during dry run! "
                f"Before: {checkpoints_count_before}, After: {checkpoints_count_after}"
            )
            return False

        # Verify chainstate slot unchanged
        chainstate_after = self.get_chainstate(tip_block_id)
        current_slot_after = chainstate_after.get("current_slot", 0)
        if current_slot_before != current_slot_after:
            self.error(
                f"Chainstate was modified during dry run! "
                f"Before: {current_slot_before}, After: {current_slot_after}"
            )
            return False

        self.info("All database counts verified unchanged despite -c -d flags")

        # Final verification: check that blocks are still accessible
        self.info("\n=== Final verification ===")
        self.info("Verifying blocks that would be deleted are still accessible")

        # Try to get chainstate at the target block
        try:
            self.get_chainstate(target_block_id)
            self.info(f"Successfully read chainstate at target block {target_block_id}")
        except Exception as e:
            self.error(f"Failed to read chainstate at target block after dry runs: {e}")
            return False

        # Try to get L2 block at tip
        try:
            self.get_l2_block(tip_block_id)
            self.info(f"Successfully read L2 block at tip {tip_block_id}")
        except Exception as e:
            self.error(f"Failed to read L2 block at tip after dry runs: {e}")
            return False

        # Verify the specific checkpoint is still accessible
        try:
            checkpoint = self.get_checkpoint(latest_checkpt_idx)
            if checkpoint.get("checkpoint"):
                self.info(f"Successfully read checkpoint at index {latest_checkpt_idx}")
            else:
                self.error(f"Checkpoint at index {latest_checkpt_idx} was deleted during dry run")
                return False
        except Exception as e:
            self.error(
                f"Failed to read checkpoint at index {latest_checkpt_idx} after dry run: {e}"
            )
            return False

        self.info("\nSuccessfully verified dry run behavior:")
        self.info("  - Command executed successfully with -c -d flags (no -f)")
        self.info("  - L2 blocks count unchanged")
        self.info("  - Checkpoints count unchanged")
        self.info("  - Chainstate slot unchanged")
        self.info("  - Target blocks remain accessible")
        self.info("  - Checkpoint remains accessible")

        return True

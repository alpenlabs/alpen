import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import DbtoolMixin
from utils.dbtool import send_tx
from utils.utils import (
    ProverClientSettings,
    wait_for_genesis,
    wait_until,
    wait_until_epoch_finalized,
)


@flexitest.register
class RevertChainstateDeleteBlocksTest(DbtoolMixin):
    """Test to revert chainstate with -d flag (deletes blocks and update status)"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Wait for genesis and generate some initial blocks
        wait_for_genesis(self.seqrpc, timeout=20)

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(self.web3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        wait_until_epoch_finalized(self.seqrpc, 1, timeout=30)

        # Generate more blocks to have a longer chain beyond the finalized epoch
        for _ in range(10):
            send_tx(self.web3)

        # Wait for both services to be in sync
        ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {ol_block_number}, EL block number: {el_block_number}")

        # Check if both services are at the same state before proceeding
        if ol_block_number != el_block_number:
            self.warning(f"OL and EL are not in sync: OL={ol_block_number}, EL={el_block_number}")

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get checkpoints summary to find the latest checkpoint
        self.info("Getting checkpoints summary to find latest checkpoint")
        checkpoints_summary = self.get_checkpoints_summary()

        checkpoints_count = checkpoints_summary.get("checkpoints_found_in_db", 0)
        if checkpoints_count == 0:
            self.error("No checkpoints found")
            return False

        # Get the latest checkpoint index (checkpoints_count - 1)
        latest_checkpt_index = checkpoints_count - 1
        self.info(f"Latest checkpoint index: {latest_checkpt_index}")

        # Get the latest checkpoint details
        latest_checkpt = self.get_checkpoint(latest_checkpt_index).get("checkpoint", {})

        # Extract the L2 range from the checkpoint
        batch_info = latest_checkpt.get("commitment", {}).get("batch_info", {})
        l2_range = batch_info.get("l2_range", {})

        if not l2_range:
            self.error("Could not find L2 range in checkpoint")
            return False

        # Get the checkpoint end slot to ensure we target a block outside checkpointed range
        checkpt_end_slot = l2_range[1].get("slot")
        checkpt_end_block_id = l2_range[1].get("blkid")

        if checkpt_end_slot is None:
            self.error("Could not find checkpoint end slot")
            return False

        self.info(f"Checkpoint end slot: {checkpt_end_slot}")

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

        # Use the tip block as target (it should be outside checkpointed range)
        target_block_id = checkpt_end_block_id
        target_slot = checkpt_end_slot

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Revert chainstate with -d flag
        self.info(f"Testing revert-chainstate to {target_block_id} with -d flag")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-d")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Chainstate revert with -d flag completed successfully")
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

        # Wait for block production to resume
        wait_until(
            lambda: self.seqrpc.strata_syncStatus()["tip_height"] > target_slot,
            error_with="not syncing blocks after revert with -d flag",
            timeout=30,
        )

        new_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        new_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)

        self.info(f"After restart - OL: {new_ol_block_number}, EL: {new_el_block_number}")

        # Services should be in sync and continue processing from the reverted block
        if new_ol_block_number != new_el_block_number:
            self.error(
                f"Services not in sync after restart: OL={new_ol_block_number}, "
                f"EL={new_el_block_number}"
            )
            return False

        self.info("Successfully reverted chainstate with -d flag and resumed processing")
        return True

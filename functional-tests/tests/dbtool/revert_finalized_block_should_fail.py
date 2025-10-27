import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import SequencerDbtoolMixin
from utils.dbtool import setup_revert_chainstate_test
from utils.utils import ProverClientSettings


@flexitest.register
class RevertFinalizedBlockShouldFailTest(SequencerDbtoolMixin):
    """Test that reverting a finalized block fails as expected"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Setup: generate blocks and finalize epoch
        setup_revert_chainstate_test(self)

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        self.info("Testing that reverting a finalized block fails")

        # Get syncinfo to find finalized epoch
        self.info("Getting syncinfo to find finalized epoch")
        sync_info = self.get_syncinfo()

        finalized_epoch = sync_info.get("finalized_epoch", {})
        finalized_epoch_last_slot = finalized_epoch.get("last_slot", 0)
        finalized_epoch_last_blkid = finalized_epoch.get("last_blkid", "")

        self.info(f"Finalized epoch last slot: {finalized_epoch_last_slot}")

        if finalized_epoch_last_slot == 0:
            self.info("No finalized epoch yet, skipping this test")
            return True

        # Target a block that's BEFORE the finalized epoch (should fail)
        target_slot = finalized_epoch_last_slot - 1
        self.info(f"Targeting slot {target_slot}")

        # Get the L2 block to find the previous block ID
        l2_block_info = self.get_l2_block(finalized_epoch_last_blkid)

        # First level header is a SignedL2BlockHeader, then L2BlockHeader
        l2_block_header = l2_block_info.get("header", {}).get("header", {})
        target_block_id = l2_block_header.get("prev_block")

        if not target_block_id:
            self.error("No previous block ID found in L2 block info")
            return False

        # Try to revert to target_block_id (should fail)
        self.info(
            f"Attempting to revert to slot: {target_slot}, "
            f"block id: {target_block_id} (should fail)"
        )
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-f")

        # The command should fail with an error
        if return_code == 0:
            self.error("revert-chainstate should have failed but succeeded")
            self.error(f"Stderr: {stdout}")
            return False

        self.info("Reverting to a block inside finalized epoch fails as expected")
        self.info(f"Stderr: {stderr}")
        return True

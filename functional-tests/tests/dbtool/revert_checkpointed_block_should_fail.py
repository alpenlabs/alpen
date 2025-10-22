import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import SequencerDbtoolMixin
from utils.dbtool import get_latest_checkpoint, setup_revert_chainstate_test, target_start_of_epoch
from utils.utils import ProverClientSettings


@flexitest.register
class RevertCheckpointedBlockShouldFailTest(SequencerDbtoolMixin):
    """Test to revert chainstate to a block inside a checkpointed epoch (should fail)"""

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

        # Get checkpoint info and target block
        checkpt = get_latest_checkpoint(self)
        if not checkpt:
            return False

        # Get a block within the checkpointed range (use the first block in the range)
        checkpt_start_block_id, checkpt_start_slot = target_start_of_epoch(checkpt["l2_range"])

        self.info(
            f"Checkpoint start slot: {checkpt_start_slot}, block ID: {checkpt_start_block_id}"
        )

        # Try to revert to a checkpointed block - this should fail
        self.info(f"Testing revert to block {checkpt_start_block_id} (should fail)")
        return_code, stdout, stderr = self.revert_chainstate(checkpt_start_block_id, "-f")

        if return_code == 0:
            self.error("revert-chainstate should have failed but succeeded")
            self.error(f"Stdout: {stdout}")
            return False

        self.info(f"revert-chainstate correctly failed with return code {return_code}")
        self.info(f"Stderr: {stderr}")

        self.info("Successfully verified that reverting to checkpointed block fails")
        return True

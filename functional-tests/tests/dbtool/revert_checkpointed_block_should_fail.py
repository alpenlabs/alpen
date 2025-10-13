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

        # Capture sync status
        ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {ol_block_number}, EL block number: {el_block_number}")

        if ol_block_number != el_block_number:
            self.warning(f"OL and EL are not in sync: OL={ol_block_number}, EL={el_block_number}")

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get checkpoint info and target block
        checkpt = get_latest_checkpoint(self)
        if not checkpt:
            return False

        # Target the START of the epoch (first block in the checkpointed range)
        target_block_id, target_slot = target_start_of_epoch(checkpt["l2_range"])
        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Try to revert to a checkpointed block WITHOUT -c flag - this should fail
        return_code, stdout, stderr = self.revert_chainstate(target_block_id)

        # The command should fail with an error
        if return_code == 0:
            self.error("revert-chainstate should have failed but succeeded")
            self.error(f"Stdout: {stdout}")
            return False

        self.info(f"revert-chainstate correctly failed with return code {return_code}")
        self.info(f"Stderr: {stderr}")

        self.info("Successfully verified that reverting to checkpointed block fails")
        return True

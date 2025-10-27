import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import FullnodeDbtoolMixin
from utils.dbtool import (
    get_latest_checkpoint,
    restart_fullnode_after_revert,
    setup_revert_chainstate_test,
    target_start_of_epoch,
    verify_checkpoint_deleted,
    verify_revert_success,
)
from utils.utils import ProverClientSettings


@flexitest.register
class RevertCheckpointedBlockFnTest(FullnodeDbtoolMixin):
    """Test revert checkpointed block on fullnode"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.HubNetworkEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Setup: generate blocks, finalize epoch 1
        setup_revert_chainstate_test(
            self,
            seqrpc_attr="seqrpc",
            web3_attr="web3",
            additional_txs=10,
        )

        # Capture state before revert
        old_seq_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        old_seq_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(
            f"Sequencer OL block number: {old_seq_ol_block_number}, "
            f"EL block number: {old_seq_el_block_number}"
        )

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

        # Get checkpoint info and target block
        checkpt = get_latest_checkpoint(self)
        if not checkpt:
            return False

        # Get epoch summary before revert
        epoch_summary = self.get_epoch_summary(checkpt["idx"])
        self.info(f"Epoch summary before revert: {epoch_summary}")

        # Use start of epoch as target (first block in checkpointed range)
        target_block_id, target_slot = target_start_of_epoch(checkpt["l2_range"])
        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Try to revert to a checkpointed block with -c and -f flags - this should succeed
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-c", "-f")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info(f"revert-chainstate succeeded with return code {return_code}")
        self.info(f"Stdout: {stdout}")

        # Verify chainstate and checkpoint data
        if not verify_revert_success(self, target_block_id, target_slot):
            return False

        # When reverting to the BEGINNING of a checkpointed epoch, checkpoint should be deleted
        if not verify_checkpoint_deleted(self, checkpt["idx"]):
            return False

        # Restart services and verify
        restart_fullnode_after_revert(self, target_slot, old_seq_ol_block_number, checkpt["idx"])

        self.info("Successfully reverted full node chainstate and verified resync")
        return True

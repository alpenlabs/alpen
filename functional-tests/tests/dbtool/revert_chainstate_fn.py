import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import FullnodeDbtoolMixin
from utils.dbtool import (
    get_latest_checkpoint,
    restart_fullnode_after_revert,
    setup_revert_chainstate_test,
    target_end_of_epoch,
    verify_checkpoint_preserved,
    verify_revert_success,
)
from utils.utils import ProverClientSettings


@flexitest.register
class RevertChainstateFnTest(FullnodeDbtoolMixin):
    """Test revert chainstate on fullnode"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.HubNetworkEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Setup: generate blocks and finalize epoch
        setup_revert_chainstate_test(self, seqrpc_attr="seqrpc", web3_attr="web3")

        # Capture state before revert
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

        target_block_id, target_slot = target_end_of_epoch(checkpt["l2_range"])

        # Ensure we have blocks outside checkpointed range
        sync_info = self.get_syncinfo()
        tip_slot = sync_info.get("l2_tip_height")

        if tip_slot and tip_slot <= target_slot:
            self.info("No blocks outside checkpointed range - test cannot proceed")
            return True

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Execute revert chainstate
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-f")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Revert chainstate completed successfully")
        self.info(f"Stdout: {stdout}")

        # Verify chainstate and checkpoint data
        if not verify_revert_success(self, target_block_id, target_slot):
            return False

        if not verify_checkpoint_preserved(self, checkpt["idx"]):
            return False

        # Restart services and verify
        restart_fullnode_after_revert(self, target_slot, old_seq_ol_block_number, checkpt["idx"])

        self.info("Successfully reverted full node chainstate and verified resync")
        return True

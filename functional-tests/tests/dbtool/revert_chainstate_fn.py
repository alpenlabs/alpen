import time

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
from utils.utils import wait_until


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
        # Setup: generate blocks and finalize epoch
        setup_revert_chainstate_test(self, web3_attr="web3")

        cur_block = int(self.rethrpc.eth_blockNumber(), base=16)

        # ensure there are some blocks more than our tip height
        wait_until(
            lambda: int(self.rethrpc.eth_blockNumber(), base=16) > cur_block + 3,
            error_with="not building blocks",
            timeout=10,
        )

        # Stop signer early to ensure no more blocks
        self.seq_signer.stop()

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
        self.seq.stop()
        self.reth.stop()
        self.follower_1_node.stop()
        self.follower_1_reth.stop()

        # extra buffer time to let latest checkpoint get final
        time.sleep(2)
        # Get checkpoint info and target block
        checkpt = get_latest_checkpoint(self)
        if not checkpt:
            return False

        target_block_id, target_slot = target_end_of_epoch(checkpt["l2_range"])

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Execute revert chainstate to the last L2 block of checkpointed range
        self.info(f"Testing revert-chainstate to {target_block_id} using fullnode (fn) database")
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

        # Get final block numbers for verification
        seq_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        seq_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        fn_ol_block_number = self.follower_1_rpc.strata_syncStatus()["tip_height"]
        fn_el_block_number = int(self.follower_1_reth_rpc.eth_blockNumber(), base=16)

        self.info(f"After restart - Sequencer OL: {seq_ol_block_number}, EL: {seq_el_block_number}")
        self.info(f"After restart - Fullnode OL: {fn_ol_block_number}, EL: {fn_el_block_number}")

        # Check sync status (warning only)
        if seq_ol_block_number != seq_el_block_number:
            self.warning(
                f"Sequencer services not in sync after restart: OL={seq_ol_block_number}, "
                f"EL={seq_el_block_number}"
            )

        if fn_ol_block_number != fn_el_block_number:
            self.warning(
                f"Fullnode services not in sync after restart: OL={fn_ol_block_number}, "
                f"EL={fn_el_block_number}"
            )

        self.info("Successfully reverted full node chainstate and verified resync")
        return True

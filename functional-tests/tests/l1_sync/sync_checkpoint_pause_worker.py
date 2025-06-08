import flexitest
import logging
from envs import net_settings, testenv

from tests.l1_sync.common import assert_ckpt_and_seq_sync, get_latest_slot
from utils import *


@flexitest.register
class SyncCheckpointPauseWorkerTest(testenv.StrataTester):
    def __init__(self, ctx: flexitest.InitContext):
        premine_blocks = 101
        settings = net_settings.get_fast_batch_settings()
        settings.genesis_trigger = premine_blocks + 5

        ctx.set_env(
            testenv.HubNetworkEnvConfig(
                premine_blocks,
                rollup_settings=settings,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        sequencer_sync_node = ctx.get_service("follower_1_node")
        checkpoint_sync_node = ctx.get_service("fullnode_ckpt")

        ss_node_rpc = sequencer_sync_node.create_rpc()
        cs_node_rpc = checkpoint_sync_node.create_rpc()

        # wait until the nodes start
        wait_until(cs_node_rpc.strata_protocolVersion, timeout=5)

        # wait for an epoch to be confirmed
        wait_until_epoch_finalized(cs_node_rpc, 0, timeout=30)
        ckpt_sync_slot_before_pause = get_latest_slot(cs_node_rpc)
        logging.info(
            f"checkpoint sync worker chainstate latest slot: {ckpt_sync_slot_before_pause}"
        )

        # stop checkpoint sync worker
        logging.info("stopping checkpoint sync worker")
        paused = cs_node_rpc.debug_pause_resume("CheckpointSyncWorker", "PauseUntilResume")

        # how do we know that we were actually succesful in pausing checkpoint sync worker?
        assert paused, "Should pause the checkpoint sync worker"

        # wait until the epoch is finalized for ss node
        wait_until_epoch_finalized(ss_node_rpc, 2, timeout=60)

        logging.info("Assert that checkpoint sync worker is paused")
        ckpt_sync_paused_at_slot = get_latest_slot(cs_node_rpc)
        assert ckpt_sync_paused_at_slot == ckpt_sync_slot_before_pause, (
            "Failed to pause checkpoint sync worker"
        )

        # resume checkpoint sync worker
        resumed = cs_node_rpc.debug_pause_resume("CheckpointSyncWorker", "Resume")
        assert resumed, "Should resume the checkpoint sync worker"
        wait_until(cs_node_rpc.strata_protocolVersion, timeout=5)

        logging.info("waiting for CSM of checkpoint sync to finalize epoch")

        # will this be sufficient time for CSM to sync?
        wait_until_epoch_finalized(cs_node_rpc, 2, timeout=60)

        # assert both nodes have synced to the same state
        assert_ckpt_and_seq_sync(cs_node_rpc=cs_node_rpc, ss_node_rpc=ss_node_rpc)

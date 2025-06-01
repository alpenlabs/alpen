import flexitest
import logging
from envs import net_settings, testenv

from utils import *


@flexitest.register
class SyncCheckpointTest(testenv.StrataTester):
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

        # ss_node = sequencer sync node
        # cs_node = checkpoint sync node
        ss_node_rpc = sequencer_sync_node.create_rpc()
        cs_node_rpc = checkpoint_sync_node.create_rpc()

        for epoch in range(0, 3):
            wait_until_epoch_finalized(cs_node_rpc, epoch, timeout=120)

            # assert both clients have processed the same number of checkpoints
            cs_ckpt_idx = cs_node_rpc.strata_getLatestCheckpointIndex()
            ss_ckpt_idx = ss_node_rpc.strata_getLatestCheckpointIndex()
            assert ss_ckpt_idx == cs_ckpt_idx

            # sequencer sync client gets to a chainstate much later than checkpoint sync client
            # just because it keeps updating chainstate based on l2 blocks it receives from sequencer
            ckpt_sync_latest_slot = cs_node_rpc.strata_getLatestChainstateSlot()
            assert ckpt_sync_latest_slot > 0 # ensure checkpoint sync client is not stuck at genesis
            logging.info(f"chain tip slot for checkpoint sync client: {ckpt_sync_latest_slot}")

            cs_chs = cs_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)
            ss_chs = ss_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)

            logging.info(f"comparing chainstates for latest slot: {ckpt_sync_latest_slot}")
            assert cs_chs == ss_chs

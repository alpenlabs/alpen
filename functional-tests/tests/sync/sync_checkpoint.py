import flexitest
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

        ss_node_rpc = sequencer_sync_node.create_rpc()
        cs_node_rpc = checkpoint_sync_node.create_rpc()

        wait_until_epoch_finalized(ss_node_rpc, 1, timeout=100)

        ckpt_sync_latest_slot = cs_node_rpc.strata_getLatestChainstateSlot()
        # sequencer sync client gets to a chainstate much later than checkpoint sync client
        # just because it keeps updating chainstate based on l2 blocks it receives from sequencer
        cs_chs = cs_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)
        ss_chs = ss_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)
        # assert that the latest chainstate for checkpoint sync is the same as the
        # chainstate for sequencer sync for the corresponding slot
        assert cs_chs == ss_chs

import flexitest
import logging
from envs import net_settings, testenv

from tests.l1_sync.common import assert_ckpt_and_seq_sync
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
            assert_ckpt_and_seq_sync(cs_node_rpc=cs_node_rpc, ss_node_rpc=ss_node_rpc)

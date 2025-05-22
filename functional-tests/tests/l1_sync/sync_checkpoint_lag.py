import flexitest
import logging
from envs import net_settings, testenv

from tests.l1_sync.common import assert_ckpt_and_seq_sync
from utils import *


@flexitest.register
class SyncCheckpointLagTest(testenv.StrataTester):
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

        # wait for an epoch to be finalized
        wait_until_epoch_finalized(cs_node_rpc, 0, timeout=30)

        # stop checkpoint sync node - does this even work??
        logging.info("stopping checkpoint sync node")
        checkpoint_sync_node.stop()

        logging.info("waiting for epoch 2 to be finalized for sequencer sync node")
        wait_until_epoch_finalized(ss_node_rpc, 2, timeout=60)

        # restart cs node
        logging.info("restarting checkpoint sync node")
        checkpoint_sync_node.start()
        wait_until(cs_node_rpc.strata_protocolVersion, timeout=5)

        logging.info("waiting for checkpoint sync node to catch up to sequencer sync node")
        wait_until_epoch_finalized(cs_node_rpc, 2, timeout=60)

        assert_ckpt_and_seq_sync(cs_node_rpc=cs_node_rpc, ss_node_rpc=ss_node_rpc)

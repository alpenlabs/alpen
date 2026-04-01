"""
Tests a checkpoint syncing node correctly updates its state based on L1.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config.constants import ServiceType
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointSyncNode(StrataNodeTest):
    """Tests that the checkpoint sync node correctly syncs from L1"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        # Get strata nodes(sequencer and non-sequencer)
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        strata_node: StrataService = self.get_service(ServiceType.StrataNode)

        # Wait for RPCs to be ready
        seqrpc = sequencer.wait_for_rpc_ready(timeout=10)
        noderpc = strata_node.wait_for_rpc_ready(timeout=10)

        # Just check that the sync status tip for node is being updated and
        # for each change, make sure sequencer has that OL block.

        num_epochs_to_check = 4
        initial_status = strata_node.get_sync_status(
            noderpc
        )  # XXX: not quite liking to have rpc passed as an arugment, allows for wrong rpc to be passed in
        tip = initial_status["tip"]
        cur_tip_epoch_node = tip["epoch"]
        for _ in range(num_epochs_to_check):
            sync_status = wait_until_with_value(
                lambda: strata_node.get_sync_status(noderpc),
                lambda v, prev=cur_tip_epoch_node: (
                    v["tip"] is not None and v["tip"]["epoch"] > prev
                ),
                timeout=60,
                error_with="Epoch not progressing",
            )
            assert tip["blkid"] != "00" * 32
            cur_tip_epoch = tip["epoch"]
            # TODO: Check that the tip, confirmed and finalized are all same
            tip_blk = sync_status["tip"]["blkid"]
            confirmed_block = sync_status["confirmed"]["last_blkid"]
            finalized_block = sync_status["finalized"]["last_blkid"]

            assert tip_blk == confirmed_block, (
                "Tip block should equal confirmed block for checkpoint node"
            )
            assert tip_blk == finalized_block, (
                "Tip block should equal finalized block for checkpoint node"
            )

            logger.info("tip epoch advanced to %s", cur_tip_epoch)

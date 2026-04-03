"""
Tests a checkpoint syncing node correctly updates its state based on L1.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config.constants import ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointSyncNode(StrataNodeTest):
    """Tests that the checkpoint sync node correctly syncs from L1"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        # Get strata nodes(sequencer and non-sequencer)
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        strata_node: StrataService = self.get_service(ServiceType.StrataNode)

        btc: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btcrpc = btc.create_rpc()

        # Wait for RPCs to be ready
        sequencer.wait_for_rpc_ready(timeout=10)
        strata_node.wait_for_rpc_ready(timeout=10)

        num_epochs_to_check = 3
        for epoch in range(1, num_epochs_to_check + 1):
            # Check finalization in sequencer
            sequencer.wait_until_checkpoint_finalized(epoch, btcrpc, timeout=120)
            logger.info(f"Epoch {epoch} finalized in sequencer")

            # Check finalization in strata node
            sync_status = strata_node.wait_until_checkpoint_finalized(epoch, timeout=20)
            logger.info(f"Epoch {epoch} finalized in strata node")

            # Check tip, confirmed and finalized are aligned in node
            tip_blk = sync_status["tip"]["blkid"]
            confirmed_block = sync_status["confirmed"]["last_blkid"]
            finalized_block = sync_status["finalized"]["last_blkid"]

            assert tip_blk == confirmed_block, (
                "Tip block should equal confirmed block for checkpoint node"
            )
            assert tip_blk == finalized_block, (
                "Tip block should equal finalized block for checkpoint node"
            )

            logger.info("finalized epoch advanced to %s", sync_status["finalized"]["epoch"])

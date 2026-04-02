"""
Verify that signed checkpoints are posted to L1 and the ASM finalizes them.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointFinalized(StrataNodeTest):
    """Signed checkpoint posted to L1, ASM finalizes it, finalized epoch advances."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        # Wait for RPC to be ready
        logger.info("Waiting for Strata RPC to be ready...")
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)

        btc_rpc = bitcoin.create_rpc()

        # Get initial sync status
        initial_status = strata.get_sync_status(strata_rpc)
        logger.info(
            "initial finalized cursor epoch %s (genesis baseline)",
            initial_status["finalized"]["epoch"],
        )

        epochs_to_check = 3

        for target_epoch in range(1, epochs_to_check + 1):
            status = strata.wait_until_checkpoint_finalized(target_epoch, btc_rpc)
            epoch = status["finalized"]
            logger.info("finalized epoch advanced to %s", epoch["epoch"])

        return True

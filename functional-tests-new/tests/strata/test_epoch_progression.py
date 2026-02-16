"""Test sequencer epoch progression."""

import logging

from common.wait import wait_until_with_value
import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerEpochProgression(StrataNodeTest):
    """Test that sequencer is correctly progressing epoch."""

    # NOTE: This test should be covered when we actually do more integrated
    # test with alpen-client because alpen-client also needs epoch progression to work properly

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        # Get sequencer service
        strata = self.get_service(ServiceType.Strata)

        # Wait for RPC to be ready
        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Get initial sync status
        initial_status = strata.get_sync_status(rpc)
        cur_epoch = initial_status["confirmed"]

        epochs_to_check = 3

        for _ in range(1, epochs_to_check + 1):
            epoch = wait_until_with_value(
                lambda: strata.get_sync_status(rpc)["confirmed"],
                lambda v, cur_epoch=cur_epoch: v is not None and v["epoch"] > cur_epoch["epoch"],
                timeout=10,
                error_with="Can't get sync status",
            )
            print("epoch", epoch)
            cur_epoch = epoch

        return True

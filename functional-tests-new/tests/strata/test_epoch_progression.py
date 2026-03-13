"""Test sequencer epoch progression."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerEpochProgression(StrataNodeTest):
    """Test that sequencer is correctly progressing epochs."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)

        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=10)

        initial_status = strata.get_sync_status(rpc)
        latest_epoch = initial_status["latest"]
        logger.info("initial latest epoch %s", latest_epoch["epoch"])
        assert latest_epoch["last_blkid"] != "00" * 32

        epochs_to_check = 3

        for _ in range(epochs_to_check):
            epoch = wait_until_with_value(
                lambda: strata.get_sync_status(rpc)["latest"],
                lambda v, cur_epoch=latest_epoch: v is not None and v["epoch"] > cur_epoch["epoch"],
                timeout=10,
                error_with="Latest epoch not progressing",
            )
            logger.info("latest epoch advanced to %s", epoch["epoch"])
            assert epoch["last_blkid"] != "00" * 32
            latest_epoch = epoch

        return True

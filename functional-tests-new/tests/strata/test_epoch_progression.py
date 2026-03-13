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
        tip = initial_status["tip"]
        cur_tip_epoch = tip["epoch"]
        logger.info("initial tip epoch %s", cur_tip_epoch)
        assert tip["blkid"] != "00" * 32

        epochs_to_check = 3

        for _ in range(epochs_to_check):
            tip = wait_until_with_value(
                lambda: strata.get_sync_status(rpc)["tip"],
                lambda v, prev=cur_tip_epoch: v is not None and v["epoch"] > prev,
                timeout=60,
                error_with="Epoch not progressing",
            )
            assert tip["blkid"] != "00" * 32
            cur_tip_epoch = tip["epoch"]
            logger.info("tip epoch advanced to %s", cur_tip_epoch)

        return True

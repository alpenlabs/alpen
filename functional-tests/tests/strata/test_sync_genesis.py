"""Test that the chain reaches genesis and produces blocks consistently."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestSyncGenesis(StrataNodeTest):
    """Verify chain reaches genesis and tip advances across multiple intervals."""

    NUM_CHECKS = 5

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Verify genesis is reached — chain status is available with non-zero tip
        status = strata.get_sync_status(rpc)
        tip = status["tip"]
        assert tip["slot"] >= 0, "Chain status not available — genesis not reached"
        assert tip["blkid"] != "00" * 32, "Tip block ID is zero — genesis not observed"
        logger.info(f"Genesis reached: slot={tip['slot']}, epoch={tip['epoch']}")

        # Verify tip advances across multiple polling intervals
        last_slot = tip["slot"]
        for i in range(self.NUM_CHECKS):
            strata.wait_for_block_height(last_slot + 1, rpc, timeout=15)
            status = strata.get_sync_status(rpc)
            new_tip = status["tip"]
            assert new_tip["slot"] > last_slot, (
                f"Check {i + 1}: tip did not advance ({new_tip['slot']} <= {last_slot})"
            )
            logger.info(
                f"Check {i + 1}/{self.NUM_CHECKS}: slot={new_tip['slot']}, "
                f"blkid={new_tip['blkid'][:16]}..."
            )
            last_slot = new_tip["slot"]

        logger.info(f"Chain produced blocks consistently across {self.NUM_CHECKS} intervals")
        return True

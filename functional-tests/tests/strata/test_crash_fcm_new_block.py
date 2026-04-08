"""Test sequencer recovers after crash during fork choice new block processing."""

import logging

import flexitest

from common.crash_helpers import CrashTest, crash_and_recover

logger = logging.getLogger(__name__)


@flexitest.register
class TestCrashFcmNewBlock(CrashTest):
    """Crash at fcm_new_block bail point and verify recovery."""

    def main(self, ctx):
        strata = self.get_strata()
        rpc = strata.wait_for_rpc_ready(timeout=10)

        strata.wait_for_additional_blocks(2, rpc)

        # FCM crash happens during block processing — require 2 blocks of advance
        # to confirm the chain is genuinely producing again, not just one in flight.
        result = crash_and_recover(strata, "fcm_new_block", expected_block_advance=2)

        logger.info(f"Post-recovery height: {result.post_status['tip']['slot']}")
        return True

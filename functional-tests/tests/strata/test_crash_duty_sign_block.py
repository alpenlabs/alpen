"""Test sequencer recovers after crash during block signing duty."""

import logging

import flexitest

from common.crash_helpers import CrashTest, crash_and_recover

logger = logging.getLogger(__name__)


@flexitest.register
class TestCrashDutySignBlock(CrashTest):
    """Crash the sequencer at the duty_sign_block bail point and verify recovery."""

    def main(self, ctx):
        strata = self.get_strata()
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Let the chain produce a few blocks before arming the bail.
        strata.wait_for_additional_blocks(2, rpc)

        result = crash_and_recover(strata, "duty_sign_block")

        post_tip = result.post_status["tip"]
        logger.info(f"Post-recovery height: {post_tip['slot']}, blkid: {post_tip['blkid']}")
        return True

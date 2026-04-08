"""Negative test: a bail tag that matches no bail point should NOT crash the process.

This test deliberately bypasses ``crash_helpers.crash_and_recover``: that helper
validates the tag against the live ``debug_listBailTags`` registry, which would
reject "nonexistent_tag_that_matches_nothing" up front. Here we want the
opposite — to confirm that arming a tag with no matching ``check_bail_trigger``
call site is a no-op at the Rust level.
"""

import logging

import flexitest

from common.crash_helpers import CrashTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestCrashNonexistentTag(CrashTest):
    """Verify that arming a bail tag with no matching bail point does not crash."""

    def main(self, ctx):
        strata = self.get_strata()
        rpc = strata.wait_for_rpc_ready(timeout=10)

        strata.wait_for_additional_blocks(2, rpc)
        pre_height = strata.get_cur_block_height(rpc)
        logger.info(f"Height before arming fake bail: {pre_height}")

        # Arm a tag that does not match any bail point. Bypasses the helper's
        # registry check on purpose: we are exercising the no-op semantics of
        # `check_bail_trigger` for unmatched tags.
        rpc.debug_bail("nonexistent_tag_that_matches_nothing")

        # Process should stay alive and keep producing blocks.
        strata.wait_for_additional_blocks(3, rpc)
        post_height = strata.get_cur_block_height(rpc)

        assert post_height >= pre_height + 3, (
            f"Process stopped producing blocks after non-matching bail: "
            f"{post_height} < {pre_height + 3}"
        )
        assert strata.check_status(), "Process died from non-matching bail tag"

        logger.info(f"Process alive, height: {post_height} — non-matching bail correctly ignored")
        return True

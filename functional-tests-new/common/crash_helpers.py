"""Helpers for crash injection tests against the strata sequencer.

Crash tests intentionally call ``debug_bail`` to abort the running strata
process, then restart it and verify recovery. Two structural concerns are
handled here so individual tests do not have to rediscover them:

1. **Env isolation.** The shared ``"basic"`` env is used by many tests; a
   crash test that fails partway through would leave a dead sequencer for
   the next test on basic. Crash tests use a standalone env (see
   :class:`CrashTest`) so a failure cannot poison sibling tests.

2. **Restart bookkeeping.** flexitest's ``ProcService`` does not nil
   ``self.proc`` when the underlying process aborts on its own. A subsequent
   ``start()`` would raise "already running". :func:`crash_and_recover`
   calls ``stop()`` before ``start()`` to clear that bookkeeping, even
   though ``wait_for_down`` already confirmed the process exited.

3. **Bail tag validation.** Tag strings are validated against the live
   ``debug_listBailTags`` RPC so a typo fails fast at arm time instead of
   hanging until the wait timeout.
"""

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

import flexitest

from common.bail_tags import require_known_bail_tag
from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.services import StrataService
from envconfigs.strata import StrataEnvConfig


@dataclass
class CrashRecoveryResult:
    """Sync status snapshots taken before and after the crash + recovery cycle."""

    pre_status: Any
    post_status: Any


def crash_and_recover(
    strata: StrataService,
    bail_tag: str,
    *,
    expected_block_advance: int = 1,
    after_arm: Callable[[], None] | None = None,
    crash_timeout: int = 30,
    restart_timeout: int = 20,
    recovery_timeout: int = 30,
    require_no_finalized_regression: bool = True,
    require_no_confirmed_regression: bool = True,
) -> CrashRecoveryResult:
    """Arm a bail tag, wait for the sequencer to abort, restart it, and verify
    the chain progresses past the crash point.

    Args:
        strata: The running strata service to crash.
        bail_tag: The bail tag to arm. Validated against the live
            ``debug_listBailTags`` RPC; an unknown tag fails fast.
        expected_block_advance: How many additional blocks the chain must
            produce after recovery before the call returns.
        after_arm: Optional callable invoked between arming the bail and
            waiting for the crash. Used by tests where an external action
            (e.g. mining L1 blocks) is required to actually trip the bail.
        crash_timeout: Seconds to wait for the process to die after arming.
        restart_timeout: Seconds to wait for the new process's RPC to come up.
        recovery_timeout: Seconds to wait for the chain to advance.
        require_no_finalized_regression: If True, asserts the post-recovery
            finalized epoch is >= the pre-crash finalized epoch. Disable
            only if a test does not exercise epoch finalization.
        require_no_confirmed_regression: If True, same check for confirmed
            epoch.

    Returns:
        :class:`CrashRecoveryResult` containing the pre- and post-recovery
        sync statuses so callers can do extra assertions if needed.
    """
    rpc = strata.create_rpc()

    # Validate the tag against the live registry first; better to fail at
    # arm time than to hang on wait_for_down for a tag that does not exist.
    tag = require_known_bail_tag(rpc, bail_tag)

    pre_status = strata.get_sync_status(rpc)
    pre_height: int = pre_status["tip"]["slot"]

    rpc.debug_bail(tag)

    if after_arm is not None:
        after_arm()

    strata.wait_for_down(timeout=crash_timeout)

    # ProcService.start() raises "already running" unless self.proc is nilled,
    # which only happens via stop(). The process is already dead, so this is a
    # bookkeeping reset, not a real terminate.
    strata.stop()
    strata.start()
    rpc = strata.wait_for_rpc_ready(timeout=restart_timeout)

    target_height = pre_height + expected_block_advance
    strata.wait_for_block_height(target_height, rpc, timeout=recovery_timeout)
    post_status = strata.get_sync_status(rpc)

    if post_status["tip"]["slot"] <= pre_height:
        raise AssertionError(
            f"chain did not progress after recovery: {post_status['tip']['slot']} <= {pre_height}"
        )

    if require_no_finalized_regression:
        pre_fin = pre_status.get("finalized", {}).get("epoch", 0) or 0
        post_fin = post_status.get("finalized", {}).get("epoch", 0) or 0
        if post_fin < pre_fin:
            raise AssertionError(
                f"finalized epoch regressed after recovery: {post_fin} < {pre_fin}"
            )

    if require_no_confirmed_regression:
        pre_conf = pre_status.get("confirmed", {}).get("epoch", 0) or 0
        post_conf = post_status.get("confirmed", {}).get("epoch", 0) or 0
        if post_conf < pre_conf:
            raise AssertionError(
                f"confirmed epoch regressed after recovery: {post_conf} < {pre_conf}"
            )

    return CrashRecoveryResult(pre_status=pre_status, post_status=post_status)


class CrashTest(StrataNodeTest):
    """Base class for tests that crash and recover the strata sequencer.

    Subclasses get a standalone strata env automatically. Override
    :attr:`pre_generate_blocks` to change the L1 starting height.

    A standalone env is used (instead of the shared ``"basic"`` env) so a
    test that fails partway through cannot leave a dead sequencer for the
    next test sharing the env. Sibling btcio tests use the same pattern;
    see ``test_l1_tracking.py`` and ``test_l1_reorg.py``.
    """

    pre_generate_blocks: int = 110

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=self.pre_generate_blocks))

    def get_strata(self) -> StrataService:
        return self.get_service(ServiceType.Strata)

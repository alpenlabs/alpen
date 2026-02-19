"""Checkpoint test helpers: duty polling and epoch parsing."""

import logging

from common.wait import wait_until, wait_until_with_value

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Sequencer signer checkpoint duty helpers
# ---------------------------------------------------------------------------


def wait_for_checkpoint_duty(
    rpc,
    timeout: int = 60,
    step: float = 1.0,
    min_epoch: int | None = None,
    method: str = "strata_strataadmin_getSequencerDuties",
):
    """Wait until getSequencerDuties returns a SignCheckpoint duty.

    When *min_epoch* is set, duties for earlier epochs are skipped.
    """

    def _get_duty():
        duties = rpc.call(method)
        for duty in duties:
            if isinstance(duty, dict) and "SignCheckpoint" in duty:
                if min_epoch is not None and parse_checkpoint_epoch(duty) < min_epoch:
                    return None
                return duty
        return None

    return wait_until_with_value(
        _get_duty,
        lambda duty: duty is not None,
        error_with="Timed out waiting for SignCheckpoint duty",
        timeout=timeout,
        step=step,
    )


def wait_for_no_unsigned_checkpoints(
    rpc,
    timeout: int = 60,
    step: float = 1.0,
    method: str = "strata_strataadmin_getSequencerDuties",
):
    """Wait until duties contain no SignCheckpoint entries."""

    def _no_checkpoint_duty() -> bool:
        duties = rpc.call(method)
        return all(not (isinstance(duty, dict) and "SignCheckpoint" in duty) for duty in duties)

    return wait_until(
        _no_checkpoint_duty,
        error_with="Timed out waiting for no SignCheckpoint duties",
        timeout=timeout,
        step=step,
    )


# ---------------------------------------------------------------------------
# Checkpoint payload parsing
# ---------------------------------------------------------------------------


def parse_checkpoint_epoch(duty: dict) -> int:
    """Extract epoch from SSZ-encoded CheckpointPayload (first 4 bytes = epoch u32 LE)."""
    checkpoint = duty["SignCheckpoint"]["checkpoint"]
    return int.from_bytes(bytes(checkpoint[:4]), "little")

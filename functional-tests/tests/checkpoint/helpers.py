"""Checkpoint test helpers: duty polling and epoch parsing."""

import logging

from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Sequencer signer checkpoint duty helpers
# ---------------------------------------------------------------------------


def wait_for_checkpoint_duty(
    rpc,
    timeout: int = 60,
    step: float = 1.0,
    min_epoch: int | None = None,
):
    """Wait until getSequencerDuties returns a SignCheckpoint duty.

    When *min_epoch* is set, duties for earlier epochs are skipped.
    When *min_epoch* is None, waits for duty at or beyond the next epoch.
    """
    if min_epoch is None:
        status = rpc.call("strata_getChainStatus")
        tip = status.get("tip")
        if not isinstance(tip, dict) or not isinstance(tip.get("epoch"), int):
            raise AssertionError(f"Unable to determine current epoch from chain status: {status}")

        min_epoch = tip["epoch"] + 1

    def _get_duty():
        duties = rpc.call("strata_strataadmin_getSequencerDuties")
        for duty in duties:
            if isinstance(duty, dict) and "SignCheckpoint" in duty:
                if parse_checkpoint_epoch(duty) < min_epoch:
                    continue
                return duty
        return None

    return wait_until_with_value(
        _get_duty,
        lambda duty: duty is not None,
        error_with="Timed out waiting for SignCheckpoint duty",
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

"""
Waiting utilities for test synchronization.
"""

import logging
import math
import time
from collections.abc import Callable
from typing import Any, TypeVar

from .rpc import RpcError

logger = logging.getLogger(__name__)

# Transient errors that should be retried rather than propagated.
# OSError covers ConnectionError, requests.RequestException (inherits IOError), etc.
_RETRYABLE = (RpcError, OSError)


def wait_until(
    fn: Callable[[], Any],
    error_with: str = "Timed out",
    timeout: int = 30,
    step: float = 0.5,
):
    """
    Wait until a function call returns truth value, given time step, and timeout.
    This function waits until function call returns truth value at the interval of step seconds.
    """
    for _ in range(math.ceil(timeout / step)):
        try:
            if fn():
                return
        except _RETRYABLE as e:
            logger.warning(f"caught {type(e).__name__}, will still wait for timeout: {e}")
        time.sleep(step)
    raise AssertionError(error_with)


T = TypeVar("T")


def wait_until_with_value(
    fn: Callable[..., T],
    predicate: Callable[[T], bool],
    error_with: str = "Timed out",
    timeout: int = 5,
    step: float = 0.5,
    debug=False,
) -> T:
    """
    Similar to `wait_until` but this returns the value of the function.
    This also takes another predicate which acts on the function value and returns a bool
    """
    for _ in range(math.ceil(timeout / step)):
        try:
            r = fn()
            if debug:
                print("Waiting.. current value:", r)
            if predicate(r):
                return r
        except _RETRYABLE as e:
            logger.warning(f"caught {type(e).__name__}, will still wait for timeout: {e}")

        time.sleep(step)
    raise AssertionError(error_with)


def wait_for_confirmed_epoch(
    rpc,
    target_epoch: int,
    timeout: int = 60,
    step: float = 1.0,
) -> int:
    """Wait until confirmed epoch (ASM-recorded) reaches target."""
    status = wait_until_with_value(
        lambda: rpc.call("strata_getChainStatus"),
        lambda s: s["confirmed"]["epoch"] >= target_epoch,
        error_with=f"Timed out waiting for confirmed epoch >= {target_epoch}",
        timeout=timeout,
        step=step,
    )
    return status["confirmed"]["epoch"]


def wait_for_finalized_epoch(
    rpc,
    target_epoch: int,
    timeout: int = 60,
    step: float = 1.0,
) -> int:
    """Wait until finalized epoch reaches target."""
    status = wait_until_with_value(
        lambda: rpc.call("strata_getChainStatus"),
        lambda s: s["finalized"]["epoch"] >= target_epoch,
        error_with=f"Timed out waiting for finalized epoch >= {target_epoch}",
        timeout=timeout,
        step=step,
    )
    return status["finalized"]["epoch"]

"""
Waiting utilities for test synchronization.
"""

import logging
import time
from collections.abc import Callable

logger = logging.getLogger(__name__)


def wait_until(
    condition: Callable[[], bool],
    timeout: int = 30,
    interval: float = 0.5,
    error_msg: str = "Timeout waiting for condition",
) -> None:
    """
    Wait until a condition function returns True.

    Args:
        condition: Function that returns True when condition is met
        timeout: Maximum time to wait in seconds
        interval: Time to wait between checks in seconds
        error_msg: Error message to raise if timeout is reached

    Raises:
        TimeoutError: If condition is not met within timeout

    Usage:
        wait_until(lambda: service.is_ready(), timeout=30)
        wait_until(lambda: rpc.strata_protocolVersion() == 1, timeout=10)
    """
    start = time.time()
    last_exception: Exception | None = None
    attempts = 0

    while time.time() - start < timeout:
        attempts += 1
        try:
            if condition():
                elapsed = time.time() - start
                logger.debug(f"Condition met after {elapsed:.2f}s ({attempts} attempts)")
                return
        except Exception as e:
            last_exception = e
            logger.debug(f"Condition check failed (attempt {attempts}): {e}")

        time.sleep(interval)

    elapsed = time.time() - start
    msg = f"{error_msg} (timeout={timeout}s, elapsed={elapsed:.2f}s, attempts={attempts})"

    if last_exception:
        msg += f"\nLast exception: {type(last_exception).__name__}: {last_exception}"

    logger.error(msg)
    raise TimeoutError(msg)

"""
Waiting utilities for test synchronization.
"""

import math
import time
from collections.abc import Callable
from typing import Any, TypeVar

from common.test_logging import get_test_logger


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
        except Exception as e:
            ety = type(e)
            get_test_logger().warning(f"caught exception {ety}, will still wait for timeout: {e}")
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
        except Exception as e:
            ety = type(e)
            get_test_logger().warning(f"caught exception {ety}, will still wait for timeout: {e}")

        time.sleep(step)
    raise AssertionError(error_with)

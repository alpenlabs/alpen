"""
Thread-local test logger context for use across the codebase.

Provides a global logger that's automatically set per-test by the runtime,
allowing library functions to log without tight coupling to test instances.
"""

import contextvars
import logging
from collections.abc import Generator
from contextlib import contextmanager

_current_test_logger: contextvars.ContextVar[logging.Logger | None] = contextvars.ContextVar(
    "test_logger", default=None
)


def get_test_logger() -> logging.Logger:
    """
    Get the current test's logger from anywhere in the codebase.

    Raises RuntimeError if called outside a test context.
    """
    logger = _current_test_logger.get()
    if logger is None:
        raise RuntimeError("No test logger set. Are you calling this outside a test context?")
    return logger


@contextmanager
def test_logger_context(test_name: str) -> Generator[logging.Logger, None, None]:
    """
    Context manager that sets up a test-specific logger for the duration of a test.

    Usage:
        with test_logger_context("test_foo"):
            # Now get_test_logger() will work anywhere
            run_test()
    """
    logger = logging.getLogger(f"test.{test_name}")
    token = _current_test_logger.set(logger)
    try:
        yield logger
    finally:
        _current_test_logger.reset(token)

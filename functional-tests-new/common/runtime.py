"""
Custom test runtime with logging context management.
"""

import flexitest

from common.test_logging import test_logger_context


class TestRuntimeWithLogging(flexitest.TestRuntime):
    """
    TestRuntime that sets up a thread-local logger context for each test.

    This allows library functions to call get_test_logger() from anywhere
    without needing to pass logger instances around.
    """

    def _exec_test(self, test_name: str, env):
        """Wraps test execution with logger context."""
        with test_logger_context(test_name):
            return super()._exec_test(test_name, env)

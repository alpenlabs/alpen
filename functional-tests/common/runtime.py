"""
Custom test runtime with test name tracking for logging.
"""

import flexitest

from common.test_logging import set_current_test


class TestRuntimeWithLogging(flexitest.TestRuntime):
    """
    TestRuntime that sets the current test name for automatic log tagging.

    All logs will be tagged with the current test name via a logging filter.
    """

    def _exec_test(self, test_name: str, env):
        """Wraps test execution with test name tracking."""
        set_current_test(test_name)
        try:
            result = super()._exec_test(test_name, env)
            test_instance = self.tests[test_name]["inst"]
            result_msg = getattr(test_instance, "result_msg", None)
            if result.get("status") == "OK" and result_msg is not None:
                result["msg"] = result_msg
            return result
        finally:
            set_current_test(None)

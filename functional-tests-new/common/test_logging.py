"""
Test name injection for logging across the codebase.

Provides a logging filter that automatically tags all logs with the current test name.
"""

import logging

_current_test_name: str | None = None


class TestNameFilter(logging.Filter):
    """
    Logging filter that injects current test name into all log records.
    """

    def filter(self, record: logging.LogRecord) -> bool:
        record.test_name = _current_test_name or "no-test"
        return True


def set_current_test(test_name: str | None) -> None:
    """
    Set the current test name for logging.

    This is called by the test runtime before each test execution.
    All logs will be tagged with this test name until it's cleared.
    """
    global _current_test_name
    _current_test_name = test_name

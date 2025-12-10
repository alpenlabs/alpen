"""
Base test class with common utilities.
"""

import logging
from typing import Literal, overload

import flexitest

from common.config import ServiceType
from common.services import (
    BitcoinServiceWrapper,
    StrataServiceWrapper,
)
from common.test_logging import get_test_logger


class BaseTest(flexitest.Test):
    """
    Base class for all functional tests.

    Provides:
    - Logging utilities
    - Waiting helpers
    - Common assertions

    Tests should explicitly:
    - Get services from ctx.get_service()
    - Create RPC clients
    - Set up any required state
    """

    def premain(self, ctx: flexitest.RunContext):
        """
        Things that need to be done before we run the test.
        """
        self.runctx = ctx

    @property
    def logger(self) -> logging.Logger:
        """Get the current test's logger from thread-local context."""
        return get_test_logger()

    @property
    def debug(self):
        """Log at DEBUG level."""
        return self.logger.debug

    @property
    def info(self):
        """Log at INFO level."""
        return self.logger.info

    @property
    def warning(self):
        """Log at WARNING level."""
        return self.logger.warning

    @property
    def error(self):
        """Log at ERROR level."""
        return self.logger.error

    @overload
    def get_service(self, typ: Literal[ServiceType.Bitcoin]) -> BitcoinServiceWrapper: ...

    @overload
    def get_service(self, typ: Literal[ServiceType.Strata]) -> StrataServiceWrapper: ...

    def get_service(self, typ: ServiceType):
        svc = self.runctx.get_service(typ)
        if svc is None:
            raise RuntimeError(
                f"Service '{typ}' not found. Available services: "
                f"{list(self.runctx.env.services.keys())}"  # type: ignore[union-attr]
            )
        return svc

    # Overriding here to have `self.get_service` return a `ServiceWrapper[Rpc]` without boilerplate.
    def main(self, ctx) -> bool:  # type: ignore[override]
        self.runctx = ctx
        return self.run()

    def run(self) -> bool:
        raise NotImplementedError

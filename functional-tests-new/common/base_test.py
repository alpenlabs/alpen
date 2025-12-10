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
        Set up logging for the test.
        This is the ONLY thing premain does - no service setup.
        """
        self.logger = logging.getLogger(f"test.{self.__class__.__name__}")
        self.debug = self.logger.debug
        self.info = self.logger.info
        self.warning = self.logger.warning
        self.error = self.logger.error
        self.runctx = ctx

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

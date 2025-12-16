"""
Base test class with common utilities.
"""

from typing import Literal, overload

import flexitest

from common.config import ServiceType
from common.services import (
    BitcoinService,
    StrataService,
)


class BaseTest(flexitest.Test):
    """
    Base class for all functional tests.

    Provides:
    - Logging utilities
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

    # Overriding here to have `self.get_service` return a `ServiceWrapper[Rpc]` without boilerplate.
    def main(self, ctx) -> bool:  # type: ignore[override]
        raise NotImplementedError


class StrataNodeTest(BaseTest):
    """
    Base Test class for testing strata. Assumes related services like strata, bitcoin, reth, etc.
    """

    @overload
    def get_service(self, typ: Literal[ServiceType.Bitcoin]) -> BitcoinService: ...

    @overload
    def get_service(self, typ: Literal[ServiceType.Strata]) -> StrataService: ...

    def get_service(self, typ: ServiceType):
        svc = self.runctx.get_service(typ)
        if svc is None:
            raise RuntimeError(
                f"Service '{typ}' not found. Available services: "
                f"{list(self.runctx.env.services.keys())}"  # type: ignore[union-attr]
            )
        return svc

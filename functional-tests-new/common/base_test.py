"""
Base test class with common utilities.
"""

import logging
from collections.abc import Callable
from typing import Literal, overload

import flexitest

from common.config import ServiceType
from common.rpc import JsonRpcClient
from common.services import (
    BitcoinServiceWrapper,
    StrataServiceWrapper,
)
from common.wait import wait_until


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

    def wait_for(
        self,
        condition: Callable[[], bool],
        timeout: int = 30,
        interval: float = 0.5,
        error_msg: str = "Timeout",
    ):
        """
        Convenience wrapper around wait_until.

        Usage:
            self.wait_for(lambda: service.is_ready())
            self.wait_for(lambda: rpc.strata_protocolVersion() == 1, timeout=10)
        """
        wait_until(condition, error_with=error_msg, timeout=timeout, step=interval)

    def wait_for_rpc_ready(
        self,
        rpc: JsonRpcClient,
        method: str = "strata_protocolVersion",
        timeout: int = 30,
    ):
        """
        Wait until an RPC endpoint is responding.

        Args:
            rpc: RPC client to test
            method: Method to call to check readiness
            timeout: Maximum time to wait

        Usage:
            self.wait_for_rpc_ready(strata_rpc)
            self.wait_for_rpc_ready(bitcoin_rpc, method="getblockchaininfo")
        """

        def check():
            try:
                rpc.call(method)
                return True
            except Exception:
                return False

        self.wait_for(
            check,
            timeout=timeout,
            error_msg=f"RPC not ready (method: {method})",
        )

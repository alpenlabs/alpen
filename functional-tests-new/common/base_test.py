"""
Base test class with common utilities.
"""

import logging

import flexitest

from common.rpc import JsonRpcClient
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

    def wait_for(
        self,
        condition,
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
        wait_until(condition, timeout=timeout, interval=interval, error_msg=error_msg)

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

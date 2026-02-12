"""
Strata service wrapper with Strata-specific health checks.
"""

import logging
from typing import TypedDict

from common.rpc import JsonRpcClient
from common.services.base import RpcService
from common.wait import wait_until, wait_until_with_value

logger = logging.getLogger(__name__)


class StrataProps(TypedDict):
    """Properties for Strata service."""

    rpc_port: int
    rpc_host: str
    rpc_url: str
    datadir: str
    mode: str


class StrataService(RpcService):
    """
    RpcService for Strata with health check via `strata_protocolVersion`.
    """

    props: StrataProps

    def __init__(
        self,
        props: StrataProps,
        cmd: list[str],
        stdout: str | None = None,
        name: str | None = None,
    ):
        """
        Initialize Strata service.

        Args:
            props: Strata service properties
            cmd: Command and arguments to execute
            stdout: Path to log file for stdout/stderr
            name: Service name for logging
        """
        super().__init__(dict(props), cmd, stdout, name)

    def _rpc_health_check(self, rpc):
        """Check Strata health by calling strata_protocolVersion."""
        rpc.strata_protocolVersion()

    def create_rpc(self) -> JsonRpcClient:
        if not self.check_status():
            raise RuntimeError("Service is not running")

        rpc = JsonRpcClient(self.props["rpc_url"])

        def _status_check(method: str):
            if not self.check_status():
                self._logger.warning(f"service '{self._name}' crashed before call to {method}")
                raise RuntimeError(f"process '{self._name}' crashed")

        rpc.set_pre_call_hook(_status_check)

        return rpc

    def wait_for_rpc_ready(
        self,
        method: str = "strata_protocolVersion",
        timeout: int = 30,
    ) -> JsonRpcClient:
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

        err = f"RPC not ready (method: {method})"
        rpc = self.create_rpc()

        wait_until(lambda: rpc.call(method) is not None, error_with=err, timeout=timeout)
        return rpc

    def get_cur_block_height(self, rpc: JsonRpcClient | None = None) -> int:
        """
        Get the current block height from chain status.

        Args:
            rpc: Optional RPC client. If None, creates a new one.

        Returns:
            Current block height (slot number)
        """
        if rpc is None:
            rpc = self.create_rpc()

        status = wait_until_with_value(
            rpc.strata_getChainStatus,
            lambda x: x is not None,
            error_with="Timed out getting chain status",
        )
        return status.get("latest", {}).get("slot", 0)

    def wait_for_block_height(
        self,
        target_height: int,
        rpc: JsonRpcClient | None = None,
        timeout: int = 10,
        poll_interval: float = 1.0,
    ) -> None:
        """
        Wait for the chain to reach a specific block height.

        Args:
            target_height: The block height to wait for
            rpc: Optional RPC client. If None, creates a new one.
            timeout: Maximum time to wait in seconds
            poll_interval: How often to check the height
        """
        if rpc is None:
            rpc = self.create_rpc()

        wait_until_with_value(
            lambda: rpc.strata_getChainStatus(),
            lambda status: status.get("latest", {}).get("slot", 0) >= target_height,
            error_with=f"Timeout waiting for block height {target_height}",
            timeout=timeout,
            step=poll_interval,
        )

    def wait_for_additional_blocks(
        self,
        additional_blocks: int,
        rpc: JsonRpcClient | None = None,
        timeout_per_block: int = 10,
        poll_interval: float = 1.0,
    ) -> int:
        """
        Wait for a number of new blocks to be produced from current tip.

        Args:
            additional_blocks: Number of new blocks to wait for.
            rpc: Optional RPC client. If None, creates a new one.
            timeout_per_block: Timeout budget in seconds per expected block.
            poll_interval: How often to check the height.

        Returns:
            Final block height after waiting.
        """
        if additional_blocks < 1:
            raise ValueError("additional_blocks must be >= 1")

        if rpc is None:
            rpc = self.create_rpc()

        start_height = self.get_cur_block_height(rpc)
        target_height = start_height + additional_blocks
        total_timeout = timeout_per_block * additional_blocks

        logger.info(
            "Waiting for %s new blocks (from %s to %s)...",
            additional_blocks,
            start_height + 1,
            target_height,
        )

        self.wait_for_block_height(
            target_height,
            rpc,
            timeout=total_timeout,
            poll_interval=poll_interval,
        )
        return self.get_cur_block_height(rpc)

    def check_block_generation_in_range(self, rpc: JsonRpcClient, start: int, end: int) -> int:
        """Checks for range of blocks produced and returns current block height"""
        logger.info(f"Waiting for blocks from {start} to {end} be produced...")
        for target_height in range(start, end + 1):
            logger.info(f"Waiting for block {target_height}...")
            self.wait_for_block_height(target_height, rpc)
        return self.get_cur_block_height(rpc)

"""
Strata service wrapper with Strata-specific health checks.
"""

from typing import TypedDict, cast

from common.rpc import JsonRpcClient
from common.services.base import RpcService
from common.wait import wait_until


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

        err = f"RPC not ready (method: {method})"
        rpc = self.create_rpc()

        wait_until(lambda: rpc.call(method) is not None, error_with=err, timeout=timeout)

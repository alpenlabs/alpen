"""
Strata service wrapper with Strata-specific health checks.
"""

from common.rpc import JsonRpcClient
from common.services.base import ServiceWrapper
from common.wait import wait_until


class StrataServiceWrapper(ServiceWrapper[JsonRpcClient]):
    """
    ServiceWrapper for Strata with health check via `strata_protocolVersion`.
    """

    def check_health(self) -> bool:
        """
        Check if Strata RPC is ready by calling strata_protocolVersion.

        Returns:
            True if Strata is running and RPC responds, False otherwise
        """
        if not self.check_status():
            return False

        try:
            rpc = self.create_rpc()
            rpc.strata_protocolVersion()
            return True
        except Exception:
            return False

    def create_rpc(self) -> JsonRpcClient:
        rpc = super().create_rpc()

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

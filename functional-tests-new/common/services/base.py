"""
Service wrapper extending flexitest.service.ProcService with standardized methods.

Avoids ad-hoc monkey-patching and provides type-safe service abstractions.
"""

import logging
from typing import Any

import flexitest

from common.wait import wait_until


class RpcService(flexitest.service.ProcService):
    """
    Extends ProcService with RPC capabilities and standardized methods for test services.

    Subclasses must implement create_rpc() to provide service-specific RPC client creation.

    Provides:
    - create_rpc() - Override to create RPC client from self.props
    - _rpc_health_check() - Override to specify health check RPC call
    - wait_for_ready() - Wait until service is healthy

    Usage:
        class MyServiceProps(TypedDict):
            rpc_url: str
            rpc_port: int

        class MyService(RpcService):
            props: MyServiceProps

            def __init__(self, props: MyServiceProps, cmd: list[str], ...):
                super().__init__(props, cmd, ...)

            def _rpc_health_check(self, rpc):
                rpc.ping()

            def create_rpc(self) -> MyRpcClient:
                return MyRpcClient(self.props["rpc_url"])

        svc = MyService(
            props={"rpc_port": 9944, "rpc_url": "ws://localhost:9944"},
            cmd=["myservice", "--flag"],
            stdout="/path/to/service.log",
            name="myservice"
        )
        svc.start()
        rpc = svc.create_rpc()
        svc.stop()
    """

    def __init__(
        self,
        props: dict[str, Any],
        cmd: list[str],
        stdout: str | None = None,
        name: str | None = None,
    ):
        """
        Initialize service wrapper.

        Args:
            props: Service properties (ports, URLs, etc.)
            cmd: Command and arguments to execute
            stdout: Path to log file for stdout/stderr
            name: Service name for logging
        """
        super().__init__(props, cmd, stdout)
        self._name = name or cmd[0]
        self._logger = logging.getLogger(f"service.{self._name}")

    def create_rpc(self):
        """
        Create RPC client for this service.

        Subclasses must override this method to provide service-specific RPC client creation.

        Returns:
            RPC client instance (type defined by subclass).

        Raises:
            NotImplementedError: If subclass doesn't implement this method
            RuntimeError: If service is not running
        """
        raise NotImplementedError("Subclass must implement create_rpc()")

    def _rpc_health_check(self, rpc: Any) -> None:
        """
        Perform RPC call to verify service health.

        Subclasses override this to call a simple RPC method that proves the service is responsive.
        This method should raise an exception if the service is unhealthy.

        Args:
            rpc: RPC client returned by create_rpc()

        Raises:
            Exception: If service is not healthy
        """
        raise NotImplementedError("Subclass must implement _rpc_health_check()")

    def check_health(self) -> bool:
        """
        Check if service is healthy and ready to accept requests.

        Checks process status and performs RPC health check via _rpc_health_check().

        Returns:
            True if service is healthy, False otherwise
        """
        if not self.check_status():
            return False

        try:
            rpc = self.create_rpc()
            self._rpc_health_check(rpc)
            return True
        except Exception:
            return False

    def wait_for_ready(self, timeout: int = 30, interval: float = 0.5) -> None:
        """
        Wait until service is healthy and ready.

        Uses check_health() to determine readiness. Override check_health()
        in subclasses for service-specific health checks.

        Args:
            timeout: Maximum time to wait in seconds
            interval: Time between health checks in seconds

        Raises:
            AssertionError: If service doesn't become ready within timeout
        """
        wait_until(
            self.check_health,
            error_with=f"Service '{self._name}' not ready",
            timeout=timeout,
            step=interval,
        )

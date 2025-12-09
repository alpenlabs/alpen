"""
Service wrapper extending flexitest.service.ProcService with standardized methods.

Avoids ad-hoc monkey-patching and provides type-safe service abstractions.
"""

import logging
from collections.abc import Callable
from dataclasses import asdict, is_dataclass
from typing import Any, Generic, TypeVar

import flexitest

from common.wait import wait_until

Rpc = TypeVar("Rpc")


class ServiceWrapper(flexitest.service.ProcService, Generic[Rpc]):
    """
    Extends ProcService with well-defined methods for test services.

    Provides standardized:
    - create_rpc() method for RPC client creation
    - Type-safe property access
    - Extensible for service-specific methods

    Usage:
        def make_rpc():
            return JsonrpcClient("ws://localhost:9944")

        svc = ServiceWrapper(
            props={"rpc_port": 9944, "rpc_url": "ws://localhost:9944"},
            cmd=["strata", "--sequencer"],
            stdout="/path/to/service.log",
            rpc_factory=make_rpc,
            name="sequencer"
        )
        svc.start()
        rpc = svc.create_rpc()
        svc.stop()
    """

    def __init__(
        self,
        props: dict[str, Any] | Any,
        cmd: list[str],
        stdout: str | None = None,
        rpc_factory: Callable[[], Rpc] | None = None,
        name: str | None = None,
    ):
        """
        Initialize service wrapper.

        Args:
            props: Service properties (ports, URLs, etc.) - dict or dataclass
            cmd: Command and arguments to execute
            stdout: Path to log file for stdout/stderr
            rpc_factory: Optional factory function to create RPC client
            name: Service name for logging
        """
        # Convert dataclass to dict if needed
        props_dict = asdict(props) if is_dataclass(props) else props  # type: ignore[args-any]

        super().__init__(props_dict, cmd, stdout)
        self._rpc_factory = rpc_factory
        self._name = name or cmd[0]
        self._logger = logging.getLogger(f"service.{self._name}")

    def create_rpc(self) -> Rpc:
        """
        Create RPC client for this service.

        Returns:
            RPC client instance (type depends on rpc_factory)

        Raises:
            NotImplementedError: If no RPC factory was configured
            RuntimeError: If service is not running
        """
        if not self._rpc_factory:
            raise NotImplementedError("No RPC factory configured for this service")
        if not self.check_status():
            raise RuntimeError("Service is not running")

        rpc = self._rpc_factory()
        return rpc

    def check_health(self) -> bool:
        """
        Check if service is healthy and ready to accept requests.

        Override this in subclasses to implement service-specific health checks.
        Default implementation just checks if process is running.

        Returns:
            True if service is healthy, False otherwise
        """
        return self.check_status()

    def wait_for_ready(self, timeout: int = 30, interval: float = 0.5) -> None:
        """
        Wait until service is healthy and ready.

        Uses check_health() to determine readiness. Override check_health()
        in subclasses for service-specific health checks.

        Args:
            timeout: Maximum time to wait in seconds
            interval: Time between health checks in seconds

        Raises:
            TimeoutError: If service doesn't become ready within timeout
        """
        wait_until(
            self.check_health,
            timeout=timeout,
            interval=interval,
            error_msg=f"Service '{self._name}' not ready",
        )

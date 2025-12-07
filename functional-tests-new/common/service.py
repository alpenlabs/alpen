"""
Service wrapper extending flexitest.service.ProcService with standardized methods.

Avoids ad-hoc monkey-patching and provides type-safe service abstractions.
"""

import logging
from typing import Any, Callable, Generic, Optional, TypeVar

import flexitest

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
        props: dict[str, Any],
        cmd: list[str],
        stdout: Optional[str] = None,
        rpc_factory: Optional[Callable[[], Rpc]] = None,
        name: Optional[str] = None,
    ):
        """
        Initialize service wrapper.

        Args:
            props: Service properties (ports, URLs, etc.)
            cmd: Command and arguments to execute
            stdout: Path to log file for stdout/stderr
            rpc_factory: Optional factory function to create RPC client
            name: Service name for logging
        """
        super().__init__(props, cmd, stdout)
        self._rpc_factory = rpc_factory
        self._name = name or cmd[0]
        self._logger = logging.getLogger(f"service.{self._name}")

    def create_rpc(self) -> Any:
        """
        Create RPC client for this service.

        Attaches a pre-call hook to check service status before every RPC call
        (if the client supports _pre_call_hook attribute).

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

        # Attach status check hook if the RPC client supports it (seqrpc does)
        if hasattr(rpc, "_pre_call_hook"):

            def _status_check(method: str):
                if not self.check_status():
                    self._logger.warning(f"service '{self._name}' crashed before call to {method}")
                    raise RuntimeError(f"process '{self._name}' crashed")

            rpc._pre_call_hook = _status_check

        return rpc

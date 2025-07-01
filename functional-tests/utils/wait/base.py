import logging
from typing import Protocol

from utils.utils import wait_until, wait_until_with_value


class WaiterProtocol(Protocol):
    """Protocol defining the interface that waiter classes must implement."""

    timeout: int
    interval: float
    logger: logging.Logger


class WaiterMixin:
    """Mixin providing wait utilities for classes implementing WaiterProtocol."""

    def _wait_until(self: WaiterProtocol, *args, timeout=None, step=None, **kwargs):
        return wait_until(
            *args,
            timeout=timeout or self.timeout,
            step=step or self.interval,
            **kwargs,
        )

    def _wait_until_with_value(self: WaiterProtocol, *args, timeout=None, step=None, **kwargs):
        return wait_until_with_value(
            *args,
            timeout=timeout or self.timeout,
            step=step or self.interval,
            **kwargs,
        )


class RpcWaiter(WaiterMixin):
    """Waiter base class that waits on a particular RPC"""

    def __init__(
        self, rpc_client, logger: logging.Logger, timeout: int = 20, interval: float = 0.5
    ):
        self.rpc_client = rpc_client
        self.logger = logger
        self.timeout = timeout
        self.interval = interval

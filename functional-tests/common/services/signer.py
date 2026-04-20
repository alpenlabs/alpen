"""
Strata-signer service wrapper.

The signer has no RPC server — it connects to a strata node's WebSocket RPC
as a client. Health is determined solely by process liveness.
"""

import logging
from typing import TypedDict

import flexitest

from common.wait import wait_until

logger = logging.getLogger(__name__)


class SignerProps(TypedDict):
    """Properties for strata-signer service."""

    datadir: str


class SignerService(flexitest.service.ProcService):
    """Process wrapper for strata-signer."""

    props: SignerProps

    def __init__(
        self,
        props: SignerProps,
        cmd: list[str],
        stdout: str | None = None,
        name: str | None = None,
    ):
        super().__init__(dict(props), cmd, stdout)
        self._name = name or "strata-signer"
        self._logger = logging.getLogger(f"service.{self._name}")

    def wait_for_ready(self, timeout: int = 10, interval: float = 0.5) -> None:
        """Wait until the signer process is running."""
        wait_until(
            self.check_status,
            error_with=f"Service '{self._name}' not ready",
            timeout=timeout,
            step=interval,
        )

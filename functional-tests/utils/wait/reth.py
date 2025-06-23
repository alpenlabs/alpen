from dataclasses import dataclass
from typing import Any

from utils.utils import wait_until_with_value


@dataclass
class RethWaiter:
    """
    Wrapper for encapsulating and waiting reth related rpcs
    """

    reth_rpc: Any
    timeout: int = 10
    interval: float = 0.5
    message: str = "Timeout: waiting for blocks"

    def wait_until_eth_block_exceeds(self, height, message: str | None = None):
        return wait_until_with_value(
            lambda: int(self.reth_rpc.eth_blockNumber(), 16),
            lambda value: value > height,
            error_with=message or self.message,
            timeout=self.timeout,
            step=self.interval,
        )

    def wait_until_eth_block_at_least(self, height, message: str | None = None):
        """
        Waits until eth block number reaches at least the specified height.
        """
        return wait_until_with_value(
            lambda: int(self.reth_rpc.eth_blockNumber(), 16),
            lambda value: value >= height,
            error_with=message or f"Timeout: waiting for block height {height}",
            timeout=self.timeout,
            step=self.interval,
        )

    def get_current_block_number(self) -> int:
        """
        Get the current block number from reth RPC.
        """
        return int(self.reth_rpc.eth_blockNumber(), 16)

"""
Bitcoin service wrapper with Bitcoin-specific health checks.
"""

from collections.abc import Callable
from typing import TypedDict, TypeVar

from bitcoinlib.services.bitcoind import BitcoindClient

from common.services.base import RpcService
from common.wait import wait_until_with_value

T = TypeVar("T")


class BitcoinProps(TypedDict):
    """Properties for Bitcoin service."""

    p2p_port: int
    rpc_port: int
    rpc_user: str
    rpc_password: str
    rpc_url: str
    datadir: str
    walletname: str


class BitcoinService(RpcService):
    """
    Rpc Service for Bitcoin with health check via `getblockchaininfo`.
    """

    props: BitcoinProps

    def __init__(
        self,
        props: BitcoinProps,
        cmd: list[str],
        stdout: str | None = None,
        name: str | None = None,
    ):
        """
        Initialize Bitcoin service.

        Args:
            props: Bitcoin service properties
            cmd: Command and arguments to execute
            stdout: Path to log file for stdout/stderr
            name: Service name for logging
        """
        super().__init__(dict(props), cmd, stdout, name)

    def _rpc_health_check(self, rpc):
        """Check Bitcoin health by calling getblockchaininfo."""
        rpc.proxy.getblockchaininfo()

    def create_rpc(self) -> BitcoindClient:
        if not self.check_status():
            raise RuntimeError("Service is not running")

        return BitcoindClient(base_url=self.props["rpc_url"], network="regtest")

    def mine_until(
        self,
        check: Callable[[], T],
        predicate: Callable[[T], bool],
        error_with: str = "Condition not met after mining",
        timeout: int = 120,
        step: float = 2.0,
        blocks_per_step: int = 1,
        mine_address: str | None = None,
    ) -> T:
        """Mine L1 blocks until a condition is satisfied.

        Evaluates ``check()`` first; if predicate is already satisfied, returns
        immediately without mining.

        Otherwise, each step mines ``blocks_per_step`` blocks and evaluates
        ``check()``. If ``mine_address`` is not provided, a fresh address is
        generated once for this call.

        Args:
            check: Function that returns the current value to evaluate.
            predicate: Predicate that determines whether the target condition is met.
            error_with: Assertion message used if the timeout is reached.
            timeout: Maximum time to wait in seconds.
            step: Polling interval in seconds between mining attempts.
            blocks_per_step: Number of blocks to mine per attempt.
            mine_address: Optional address to mine to. If omitted, one is generated.

        Returns:
            The value returned by ``check`` that satisfied ``predicate``.
        """
        if blocks_per_step < 1:
            raise ValueError("blocks_per_step must be >= 1")
        if timeout <= 0:
            raise ValueError("timeout must be > 0")
        if step <= 0:
            raise ValueError("step must be > 0")

        current = check()
        if predicate(current):
            return current

        rpc = self.create_rpc()
        mine_addr = mine_address if mine_address is not None else rpc.proxy.getnewaddress()

        def _mine_and_check():
            rpc.proxy.generatetoaddress(blocks_per_step, mine_addr)
            return check()

        return wait_until_with_value(
            _mine_and_check,
            predicate,
            error_with=error_with,
            timeout=timeout,
            step=step,
        )

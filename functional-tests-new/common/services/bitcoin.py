"""
Bitcoin service wrapper with Bitcoin-specific health checks.
"""

from typing import TypedDict

from bitcoinlib.services.bitcoind import BitcoindClient

from common.services.base import RpcService


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

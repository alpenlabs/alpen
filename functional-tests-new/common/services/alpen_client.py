"""
Alpen-client service wrapper with P2P and Ethereum RPC capabilities.
"""

import atexit
import contextlib
import logging
import subprocess
from typing import TypedDict

from common.rpc import JsonRpcClient
from common.services.base import RpcService
from common.wait import wait_until

logger = logging.getLogger(__name__)


def _register_kill(proc):
    """Register process for cleanup on exit."""

    def kill():
        with contextlib.suppress(Exception):
            proc.kill()

    atexit.register(kill)


class AlpenClientProps(TypedDict):
    """Properties for alpen-client service."""

    http_port: int
    http_url: str
    p2p_port: int
    datadir: str
    mode: str  # "sequencer" or "fullnode"
    enode: str | None


class AlpenClientService(RpcService):
    """
    RpcService for alpen-client with Ethereum JSON-RPC and P2P capabilities.
    """

    props: AlpenClientProps

    def __init__(
        self,
        props: AlpenClientProps,
        cmd: list[str],
        stdout: str | None = None,
        name: str | None = None,
        env: dict[str, str] | None = None,
    ):
        super().__init__(dict(props), cmd, stdout, name)
        self._env = env

    def start(self):
        """Start the process with optional environment variables."""
        if self.is_started():
            raise RuntimeError("already running")

        self._reset_state()

        kwargs = {}
        if self.stdout is not None:
            if isinstance(self.stdout, str):
                f = open(self.stdout, "a")  # noqa: SIM115
                f.write(f"(process started as: {self.cmd})\n")
                kwargs["stdout"] = f
                kwargs["stderr"] = f
            else:
                kwargs["stdout"] = self.stdout

        # Add environment variables if provided
        if self._env is not None:
            kwargs["env"] = self._env

        p = subprocess.Popen(self.cmd, **kwargs)
        _register_kill(p)
        self.proc = p
        self._update_status_msg()

    def _rpc_health_check(self, rpc):
        """Check health by calling eth_blockNumber."""
        rpc.eth_blockNumber()

    def create_rpc(self) -> JsonRpcClient:
        if not self.check_status():
            raise RuntimeError("Service is not running")

        rpc = JsonRpcClient(self.props["http_url"])

        def _status_check(method: str):
            if not self.check_status():
                self._logger.warning(f"service '{self._name}' crashed before call to {method}")
                raise RuntimeError(f"process '{self._name}' crashed")

        rpc.set_pre_call_hook(_status_check)

        return rpc

    def get_block_number(self) -> int:
        """Get current block number."""
        rpc = self.create_rpc()
        result = rpc.eth_blockNumber()
        return int(result, 16)

    def get_block_by_number(self, number: int | str) -> dict | None:
        """Get block by number."""
        rpc = self.create_rpc()
        if isinstance(number, int):
            number = hex(number)
        return rpc.eth_getBlockByNumber(number, False)

    def get_peers(self) -> list[dict]:
        """Get connected peers via admin_peers."""
        rpc = self.create_rpc()
        try:
            return rpc.admin_peers()
        except Exception as e:
            logger.debug(f"get_peers failed: {e}")
            return []

    def get_peer_count(self) -> int:
        """Get number of connected peers."""
        rpc = self.create_rpc()
        try:
            result = rpc.net_peerCount()
            return int(result, 16)
        except Exception as e:
            logger.debug(f"get_peer_count failed: {e}")
            return 0

    def get_node_info(self) -> dict:
        """Get node info including enode URL."""
        rpc = self.create_rpc()
        return rpc.admin_nodeInfo()

    def get_enode(self) -> str:
        """Get the enode URL for this node."""
        info = self.get_node_info()
        return info.get("enode", "")

    def wait_for_block(self, block_number: int, timeout: int = 30) -> bool:
        """
        Wait until node reaches specified block number.

        Args:
            block_number: Target block number
            timeout: Maximum time to wait in seconds

        Returns:
            True if block reached, raises on timeout
        """
        wait_until(
            lambda: self.get_block_number() >= block_number,
            error_with=f"Block {block_number} not reached",
            timeout=timeout,
        )
        return True

    def wait_for_peers(self, count: int, timeout: int = 30) -> bool:
        """
        Wait until node has at least N peers.

        Args:
            count: Minimum number of peers
            timeout: Maximum time to wait in seconds

        Returns:
            True if peer count reached, raises on timeout
        """
        wait_until(
            lambda: self.get_peer_count() >= count,
            error_with=f"Peer count {count} not reached",
            timeout=timeout,
        )
        return True

    def wait_for_block_hash(self, block_number: int, expected_hash: str, timeout: int = 30) -> bool:
        """
        Wait until node has block with expected hash.

        Args:
            block_number: Block number to check
            expected_hash: Expected block hash
            timeout: Maximum time to wait

        Returns:
            True if block hash matches
        """

        def check():
            block = self.get_block_by_number(block_number)
            if block is None:
                return False
            return block.get("hash") == expected_hash

        wait_until(
            check,
            error_with=f"Block {block_number} hash mismatch",
            timeout=timeout,
        )
        return True

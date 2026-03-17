"""
Alpen-client service wrapper with P2P and Ethereum RPC capabilities.
"""

import atexit
import contextlib
import logging
import subprocess
import time
from typing import TypedDict

from common.config.constants import (
    DEFAULT_BLOCK_WAIT_SLACK_SECONDS,
    DEFAULT_EE_BLOCK_TIME_MS,
)
from common.rpc import JsonRpcClient
from common.services.base import RpcService
from common.wait import timeout_for_expected_blocks, wait_until

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

    def get_block_wait_timeout(
        self,
        expected_blocks: int,
        timeout_per_block: float = DEFAULT_EE_BLOCK_TIME_MS / 1000,
        timeout_slack: int = DEFAULT_BLOCK_WAIT_SLACK_SECONDS,
    ) -> int:
        """Compute a timeout budget for waiting on EE blocks."""
        return timeout_for_expected_blocks(
            expected_blocks,
            seconds_per_block=timeout_per_block,
            slack_seconds=timeout_slack,
        )

    def wait_for_block(
        self,
        block_number: int,
        timeout: int | None = None,
        poll_interval: float = 0.5,
    ) -> bool:
        """
        Wait until node reaches specified block number.

        Args:
            block_number: Target block number
            timeout: Maximum time to wait in seconds. If omitted, derives
                a timeout from the remaining block gap.
            poll_interval: Time between polling attempts in seconds

        Returns:
            True if block reached, raises on timeout
        """
        current_block = self.get_block_number()
        if current_block >= block_number:
            return True

        if timeout is None:
            remaining_blocks = block_number - current_block
            timeout = self.get_block_wait_timeout(remaining_blocks)

        wait_until(
            lambda: self.get_block_number() >= block_number,
            error_with=f"Block {block_number} not reached",
            timeout=timeout,
            step=poll_interval,
        )
        return True

    def wait_for_additional_blocks(
        self,
        additional_blocks: int,
        timeout_per_block: float = DEFAULT_EE_BLOCK_TIME_MS / 1000,
        timeout_slack: int = DEFAULT_BLOCK_WAIT_SLACK_SECONDS,
        poll_interval: float = 0.5,
    ) -> int:
        """
        Wait for a number of new EE blocks from the current tip.

        Args:
            additional_blocks: Number of new blocks to wait for.
            timeout_per_block: Timeout budget per expected block.
            timeout_slack: Extra seconds to absorb startup and polling jitter.
            poll_interval: Time between polling attempts in seconds.

        Returns:
            Final block number after waiting.
        """
        if additional_blocks < 1:
            raise ValueError("additional_blocks must be >= 1")

        start_block = self.get_block_number()
        target_block = start_block + additional_blocks
        timeout = self.get_block_wait_timeout(
            additional_blocks,
            timeout_per_block=timeout_per_block,
            timeout_slack=timeout_slack,
        )

        logger.info(
            "Waiting for %s new EE blocks (from %s to %s)...",
            additional_blocks,
            start_block + 1,
            target_block,
        )
        self.wait_for_block(
            target_block,
            timeout=timeout,
            poll_interval=poll_interval,
        )
        return self.get_block_number()

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

    def wait_for_non_empty_blob(
        self,
        btc_rpc,
        mine_address: str,
        all_envelopes: list,
        end_l1: int,
        min_last_block_num: int,
        phase_name: str,
        poll_attempts: int = 4,
        blocks_per_poll: int = 2,
        pre_poll_sleep: float = 3.0,
        post_mine_sleep: float = 2.0,
    ) -> tuple[object, int, int]:
        """Mine a bounded number of L1 blocks and wait for a later non-empty DA blob.

        This helper is used by DA functional tests that need to keep scanning
        Bitcoin blocks while preserving previously observed envelopes across
        restarts or multiple posting phases.
        """
        from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes

        mined_blocks = 0
        for attempt in range(poll_attempts):
            time.sleep(pre_poll_sleep)
            btc_rpc.proxy.generatetoaddress(blocks_per_poll, mine_address)
            mined_blocks += blocks_per_poll
            time.sleep(post_mine_sleep)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.info(
                    "%s attempt %s: found %s new DA envelope(s)",
                    phase_name,
                    attempt + 1,
                    len(new_envs),
                )
                all_envelopes.extend(new_envs)
            else:
                logger.debug("%s attempt %s: no new DA envelopes", phase_name, attempt + 1)

            blob = self._find_non_empty_blob_after_block(all_envelopes, min_last_block_num)
            if blob is not None:
                logger.info(
                    "%s blob found on attempt %s: last_block_num=%s, state_diff=%s bytes",
                    phase_name,
                    attempt + 1,
                    blob.last_block_num,
                    len(blob.state_diff),
                )
                return blob, end_l1, mined_blocks

        raise AssertionError(
            f"{phase_name}: no non-empty DA blob found after block {min_last_block_num} "
            f"within {mined_blocks} mined L1 blocks"
        )

    def advance_to_next_da_window(
        self,
        additional_blocks: int,
        timeout_slack: int = 20,
    ) -> None:
        """Wait long enough for the current batch to seal and DA posting to begin."""
        self.wait_for_additional_blocks(
            additional_blocks,
            timeout_slack=timeout_slack,
        )

    @staticmethod
    def _find_non_empty_blob_after_block(envelopes: list, min_last_block_num: int):
        """Return the latest non-empty reassembled blob posted after `min_last_block_num`."""
        from tests.alpen_client.ee_da.codec import reassemble_blobs_from_envelopes

        blobs = reassemble_blobs_from_envelopes(envelopes)
        candidates = [
            blob
            for blob in blobs
            if blob.last_block_num > min_last_block_num and not blob.is_empty_batch()
        ]
        if not candidates:
            return None
        return max(candidates, key=lambda blob: blob.last_block_num)

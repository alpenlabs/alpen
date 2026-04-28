"""Test alpen_getBlockStatus RPC for EE block finality progression."""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc import RpcError
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until

logger = logging.getLogger(__name__)

# Fullnode OLTracker polls on an interval, so its consensus heads may lag the
# sequencer by a few seconds. Wait this long for cross-node convergence.
FULLNODE_SYNC_TIMEOUT = 15


@flexitest.register
class TestEeBlockStatus(BaseTest):
    STATUS_ORDER = ["pending", "confirmed", "finalized"]
    TARGET_BLOCK_NUMBER = 5

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def assert_statuses_consistent(self, alpen_seq, alpen_rpc, up_to_block: int) -> dict:
        """Assert statuses are monotonically non-increasing for non-genesis blocks.

        Collects statuses for blocks 0..N, then checks ordering only within block 1..N.
        Returns a dict mapping block number to status string.
        """
        statuses = {}
        for i in range(up_to_block + 1):
            block = alpen_rpc.eth_getBlockByNumber(hex(i), False)
            assert block is not None, f"Failed to get block {i}"
            status = alpen_seq.get_block_status(block["hash"])
            statuses[i] = status
            logger.info(f"  Block {i}: {status}")

        for i in range(2, up_to_block + 1):
            prev = self.STATUS_ORDER.index(statuses[i - 1])
            curr = self.STATUS_ORDER.index(statuses[i])
            assert prev >= curr, (
                f"Block {i - 1} ({statuses[i - 1]}) should have equal or higher status "
                f"than block {i} ({statuses[i]})"
            )

        return statuses

    def wait_for_fullnode_match(
        self,
        alpen_fullnode: AlpenClientService,
        block_hash: str,
        expected_status: str,
        timeout: int = FULLNODE_SYNC_TIMEOUT,
    ) -> None:
        """Poll the fullnode until its status for ``block_hash`` matches ``expected_status``.

        Absorbs the OLTracker polling lag between sequencer and fullnode
        without making the test flaky.
        """
        wait_until(
            lambda: alpen_fullnode.get_block_status(block_hash) == expected_status,
            error_with=(
                f"Fullnode status for {block_hash} did not converge to "
                f"{expected_status!r} within {timeout}s "
                f"(last={alpen_fullnode.get_block_status(block_hash)!r})"
            ),
            timeout=timeout,
        )

    def assert_fullnode_matches_sequencer(
        self,
        alpen_seq: AlpenClientService,
        alpen_fullnode: AlpenClientService,
        alpen_rpc,
        up_to_block: int,
    ) -> None:
        """Assert the fullnode converges to the sequencer's status for each block."""
        for i in range(up_to_block + 1):
            block = alpen_rpc.eth_getBlockByNumber(hex(i), False)
            assert block is not None, f"Failed to get block {i}"
            seq_status = alpen_seq.get_block_status(block["hash"])
            self.wait_for_fullnode_match(alpen_fullnode, block["hash"], seq_status)
            logger.info(f"  Block {i}: fullnode converged to {seq_status}")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        alpen_fullnode: AlpenClientService = self.get_service(ServiceType.AlpenFullNode)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        # Wait for chains to be active
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=20,
        )
        alpen_seq.wait_for_block(self.TARGET_BLOCK_NUMBER, timeout=60)
        alpen_fullnode.wait_for_block(self.TARGET_BLOCK_NUMBER, timeout=60)

        alpen_rpc = alpen_seq.create_rpc()

        # Non-existent block should error
        fake_hash = "0x" + "00" * 32
        try:
            alpen_rpc.alpen_getBlockStatus(fake_hash)
            raise AssertionError("Expected error for non-existent block hash")
        except RpcError as e:
            logger.info(
                "Non-existent block returned expected error: code=%s message=%s",
                e.code,
                e.message,
            )
            assert e.code == -32602, f"Expected invalid params (-32602), got {e.code}"

        # Track target block through status progression.
        target_block_hex = hex(self.TARGET_BLOCK_NUMBER)
        target_block = alpen_rpc.eth_getBlockByNumber(target_block_hex, False)
        assert target_block is not None, f"Failed to get block {self.TARGET_BLOCK_NUMBER}"
        target_hash = target_block["hash"]

        # Fullnode must also serve the method (STR-3076). Unknown hash still errors.
        fullnode_rpc = alpen_fullnode.create_rpc()
        try:
            fullnode_rpc.alpen_getBlockStatus(fake_hash)
            raise AssertionError("Expected error for non-existent block hash on fullnode")
        except RpcError as e:
            logger.info(
                "Fullnode non-existent block returned expected error: code=%s message=%s",
                e.code,
                e.message,
            )
            assert e.code == -32602, f"Expected invalid params (-32602), got {e.code}"

        initial_status = alpen_seq.get_block_status(target_hash)
        logger.info(f"Block {self.TARGET_BLOCK_NUMBER} initial status: {initial_status}")

        # Fullnode should converge to the same status for the target block.
        self.wait_for_fullnode_match(alpen_fullnode, target_hash, initial_status)

        # Block 0 should be finalized.
        block_0 = alpen_rpc.eth_getBlockByNumber("0x0", False)
        status_0 = alpen_seq.get_block_status(block_0["hash"])
        logger.info(f"Block 0 status: {status_0}")
        assert status_0 == "finalized", f"Expected finalized, got {status_0}"

        if initial_status == "finalized":
            logger.info("Initial status consistency check:")
            self.assert_statuses_consistent(
                alpen_seq, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
            )
            logger.info("Initial fullnode parity check:")
            self.assert_fullnode_matches_sequencer(
                alpen_seq, alpen_fullnode, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
            )
            logger.info(
                "Block %s is already finalized at initial check; "
                "skipping mining progression checks",
                self.TARGET_BLOCK_NUMBER,
            )
            return True

        # Mine until target block is confirmed.
        status = bitcoin.mine_until(
            check=lambda: alpen_seq.get_block_status(target_hash),
            predicate=lambda s: s in ("confirmed", "finalized"),
            error_with=(f"Block {self.TARGET_BLOCK_NUMBER} did not reach confirmed status"),
        )
        logger.info(f"Block {self.TARGET_BLOCK_NUMBER} reached: {status}")

        # Blocks 0-target should be at least confirmed.
        logger.info("Post-confirmed consistency check:")
        statuses = self.assert_statuses_consistent(
            alpen_seq, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
        )
        for i in range(self.TARGET_BLOCK_NUMBER + 1):
            assert statuses[i] in ("confirmed", "finalized"), (
                f"Block {i} should be at least confirmed, got {statuses[i]}"
            )

        logger.info("Post-confirmed fullnode parity check:")
        self.assert_fullnode_matches_sequencer(
            alpen_seq, alpen_fullnode, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
        )

        # Mine until target block is finalized.
        status = bitcoin.mine_until(
            check=lambda: alpen_seq.get_block_status(target_hash),
            predicate=lambda s: s == "finalized",
            error_with=(f"Block {self.TARGET_BLOCK_NUMBER} did not reach finalized status"),
        )
        logger.info(f"Block {self.TARGET_BLOCK_NUMBER} reached: {status}")

        # Blocks 0-target must be finalized.
        logger.info("Post-finalized consistency check:")
        statuses = self.assert_statuses_consistent(
            alpen_seq, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
        )
        for i in range(self.TARGET_BLOCK_NUMBER + 1):
            assert statuses[i] == "finalized", f"Block {i} should be finalized, got {statuses[i]}"

        logger.info("Post-finalized fullnode parity check:")
        self.assert_fullnode_matches_sequencer(
            alpen_seq, alpen_fullnode, alpen_rpc, up_to_block=self.TARGET_BLOCK_NUMBER
        )

        logger.info("Block status progression test passed")
        return True

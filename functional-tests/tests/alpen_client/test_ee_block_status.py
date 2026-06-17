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

    def assert_epoch_matches_status(self, response: dict) -> None:
        """Assert the checkpoint epoch presence and shape match the returned status.

        Pending blocks carry no ``checkpoint_epoch`` key; confirmed and finalized
        blocks carry an integer ``checkpoint_epoch`` for the OL checkpoint that
        contains the block.
        """
        status = response["status"]
        checkpoint_epoch = response.get("checkpoint_epoch")
        if status == "pending":
            assert checkpoint_epoch is None, (
                f"Pending block should not have a checkpoint_epoch, got {checkpoint_epoch!r}"
            )
        else:
            assert isinstance(checkpoint_epoch, int), (
                f"{status} block should have integer checkpoint_epoch, got {checkpoint_epoch!r}"
            )

    def assert_statuses_consistent(self, alpen_seq, alpen_rpc, up_to_block: int) -> dict:
        """Assert status and checkpoint-epoch ordering across blocks 0..N.

        Statuses must be monotonically non-increasing for non-genesis blocks, and
        the per-block containing checkpoint epoch must be monotonically
        non-decreasing with block number (a lower block cannot be checkpointed in a
        later epoch than a higher block). Genesis is checkpointed in epoch 0.

        Returns a dict mapping block number to status string.
        """
        statuses = {}
        epochs = {}
        for i in range(up_to_block + 1):
            block = alpen_rpc.eth_getBlockByNumber(hex(i), False)
            assert block is not None, f"Failed to get block {i}"
            response = alpen_seq.get_block_status(block["hash"])
            self.assert_epoch_matches_status(response)
            status = response["status"]
            statuses[i] = status
            epochs[i] = response.get("checkpoint_epoch")
            logger.info(
                "  Block %s: status=%s checkpoint_epoch=%s",
                i,
                status,
                epochs[i],
            )

        assert epochs[0] == 0, f"Genesis should be checkpointed in epoch 0, got {epochs[0]!r}"

        for i in range(2, up_to_block + 1):
            prev = self.STATUS_ORDER.index(statuses[i - 1])
            curr = self.STATUS_ORDER.index(statuses[i])
            assert prev >= curr, (
                f"Block {i - 1} ({statuses[i - 1]}) should have equal or higher status "
                f"than block {i} ({statuses[i]})"
            )

        # Containing epoch is non-decreasing with block number for checkpointed blocks.
        for i in range(1, up_to_block + 1):
            if epochs[i] is None or epochs[i - 1] is None:
                continue
            assert epochs[i - 1] <= epochs[i], (
                f"Block {i - 1} epoch ({epochs[i - 1]}) should be <= block {i} epoch ({epochs[i]})"
            )

        return statuses

    def wait_for_fullnode_match(
        self,
        alpen_fullnode: AlpenClientService,
        block_hash: str,
        expected_response: dict,
        timeout: int = FULLNODE_SYNC_TIMEOUT,
    ) -> None:
        """Poll the fullnode until its status response for ``block_hash`` matches.

        Absorbs the OLTracker polling lag between sequencer and fullnode
        without making the test flaky.
        """
        wait_until(
            lambda: alpen_fullnode.get_block_status(block_hash) == expected_response,
            error_with=(
                f"Fullnode status response for {block_hash} did not converge to "
                f"{expected_response!r} within {timeout}s "
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
            seq_response = alpen_seq.get_block_status(block["hash"])
            self.assert_epoch_matches_status(seq_response)
            self.wait_for_fullnode_match(alpen_fullnode, block["hash"], seq_response)
            logger.info("  Block %s: fullnode converged to %s", i, seq_response)

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

        initial_response = alpen_seq.get_block_status(target_hash)
        self.assert_epoch_matches_status(initial_response)
        initial_status = initial_response["status"]
        logger.info(
            "Block %s initial status response: %s",
            self.TARGET_BLOCK_NUMBER,
            initial_response,
        )

        # Fullnode should converge to the same status response for the target block.
        self.wait_for_fullnode_match(alpen_fullnode, target_hash, initial_response)

        # Block 0 should be finalized at genesis epoch 0.
        block_0 = alpen_rpc.eth_getBlockByNumber("0x0", False)
        response_0 = alpen_seq.get_block_status(block_0["hash"])
        logger.info("Block 0 status response: %s", response_0)
        assert response_0["status"] == "finalized", f"Expected finalized, got {response_0}"
        assert response_0["checkpoint_epoch"] == 0, f"Expected checkpoint_epoch 0, got {response_0}"

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
            check=lambda: alpen_seq.get_block_status(target_hash)["status"],
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
            check=lambda: alpen_seq.get_block_status(target_hash)["status"],
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

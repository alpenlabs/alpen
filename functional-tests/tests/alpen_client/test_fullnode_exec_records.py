"""Regression test for the fullnode ExEx-driven ExecBlockRecord pipeline (STR-3076).

Before STR-3076 a fullnode never wrote ExecBlockRecords: the sequencer's
block-builder was the only producer. On restart, the fullnode's exec-chain
tracker could only init from genesis and `best_finalized_block` stayed stuck
there. After STR-3076 an ExEx populates records for every canonical block
Reth imports, so the fullnode's exec-chain state is real: records persist
across restarts and the tracker resumes from the persisted tip.

This test exercises that contract end-to-end with a restart in the middle:

1. Wait for the sequencer + fullnode to produce / import blocks.
2. Mine bitcoin until finalization catches up.
3. Stop the fullnode (records should now be in EeNodeStorage).
4. Start it again (restart must NOT fail with "MissingGenesisBlock" or
   similar — `init_exec_chain_state_from_storage` must see the genesis
   record and any unfinalized records the ExEx wrote).
5. Verify the fullnode still answers `strataee_getBlockStatus` for
   historical blocks and still converges with the sequencer as the chain
   progresses.

We cannot assert `best_finalized_block > genesis` directly from outside the
process — there is no RPC surface that exposes it — but if the ExEx were
silently failing to persist records, the restart path would surface as
process crashes, stuck finality, or diverging status replies.
"""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until

logger = logging.getLogger(__name__)

# Fullnode OLTracker polls on an interval, so its consensus heads may lag the
# sequencer by a few seconds. Wait this long for cross-node convergence.
FULLNODE_SYNC_TIMEOUT = 20
# Pause between fullnode stop and start so the OS releases the datadir lock
# before we try to re-open it.
RESTART_PAUSE_SECONDS = 2
# How many blocks to produce before the restart — enough for the ExEx to
# commit a non-trivial chain of records, not so many that the test gets slow.
TARGET_BLOCK_NUMBER = 5


@flexitest.register
class TestFullnodeExecRecordsPersist(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def wait_for_fullnode_match(
        self,
        alpen_fullnode: AlpenClientService,
        block_hash: str,
        expected_status: str,
        timeout: int = FULLNODE_SYNC_TIMEOUT,
    ) -> None:
        """Poll the fullnode until its status for ``block_hash`` matches ``expected_status``."""
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
        for i in range(up_to_block + 1):
            block = alpen_rpc.eth_getBlockByNumber(hex(i), False)
            assert block is not None, f"Failed to get block {i}"
            seq_status = alpen_seq.get_block_status(block["hash"])
            self.wait_for_fullnode_match(alpen_fullnode, block["hash"], seq_status)

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        alpen_fullnode: AlpenClientService = self.get_service(ServiceType.AlpenFullNode)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=20,
        )
        alpen_seq.wait_for_block(TARGET_BLOCK_NUMBER, timeout=60)
        alpen_fullnode.wait_for_block(TARGET_BLOCK_NUMBER, timeout=60)

        alpen_rpc = alpen_seq.create_rpc()

        # Mine until the target block is finalized. This forces the ExEx to
        # have committed records AND the exec-chain tracker to have advanced
        # finalization past genesis — the key state the restart must
        # preserve.
        target_block = alpen_rpc.eth_getBlockByNumber(hex(TARGET_BLOCK_NUMBER), False)
        assert target_block is not None, f"Failed to get block {TARGET_BLOCK_NUMBER}"
        target_hash = target_block["hash"]

        logger.info("Mining bitcoin until block %s finalizes...", TARGET_BLOCK_NUMBER)
        status = bitcoin.mine_until(
            check=lambda: alpen_seq.get_block_status(target_hash),
            predicate=lambda s: s == "finalized",
            error_with=f"Block {TARGET_BLOCK_NUMBER} did not reach finalized status",
        )
        logger.info("Block %s reached: %s", TARGET_BLOCK_NUMBER, status)

        # Fullnode must converge before the restart — otherwise a successful
        # restart would not prove records persisted, just that finality
        # propagated post-restart.
        self.wait_for_fullnode_match(alpen_fullnode, target_hash, "finalized")
        logger.info("Pre-restart fullnode parity check:")
        self.assert_fullnode_matches_sequencer(
            alpen_seq, alpen_fullnode, alpen_rpc, up_to_block=TARGET_BLOCK_NUMBER
        )

        # Restart the fullnode. If the ExEx were silently failing to persist
        # records, restart would either panic on init (MissingGenesisBlock is
        # already prevented by ensure_finalized_exec_chain_genesis, but deeper
        # invariants in exec_chain_tracker_task could still trip) or come up
        # with a genesis-only exec-chain tip and fail to re-advance finality.
        pre_restart_block = alpen_fullnode.get_block_number()
        logger.info("Stopping fullnode at block %s...", pre_restart_block)
        alpen_fullnode.stop()
        time.sleep(RESTART_PAUSE_SECONDS)
        logger.info("Starting fullnode...")
        alpen_fullnode.start()
        alpen_fullnode.wait_for_ready(timeout=60)

        # After restart the fullnode must catch up to at least where it was,
        # and keep serving strataee_getBlockStatus for historical blocks. Use
        # the existing RPC as the observable — if the ExEx were broken in a
        # way that corrupted exec-chain state on replay, OLTracker-based
        # status replies would not help because the record pipeline would
        # still panic or diverge on new blocks.
        alpen_fullnode.wait_for_block(pre_restart_block, timeout=60)
        logger.info("Fullnode caught up post-restart to block %s", pre_restart_block)

        # Historical finalized status must still resolve correctly.
        self.wait_for_fullnode_match(alpen_fullnode, target_hash, "finalized")

        # Progress the chain and verify the fullnode keeps up and stays in
        # parity with the sequencer on a freshly-produced block.
        alpen_seq.wait_for_additional_blocks(3)
        new_tip = alpen_seq.get_block_number()
        alpen_fullnode.wait_for_block(new_tip, timeout=60)

        logger.info("Post-restart fullnode parity check:")
        self.assert_fullnode_matches_sequencer(
            alpen_seq, alpen_fullnode, alpen_rpc, up_to_block=new_tip
        )

        logger.info("Fullnode ExecBlockRecord persistence test passed")
        return True

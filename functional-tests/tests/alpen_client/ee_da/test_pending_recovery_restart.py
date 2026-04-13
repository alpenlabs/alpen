"""Verify pending DA envelopes recover across restart without blocking later batches."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import DaEnvelope

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaPendingRecoveryRestartTest(BaseTest):
    """
    Verify a restarted sequencer still posts later DA while older envelopes are unfinalized.

    This is accomplished by:
    1. posting a first non-empty DA batch
    2. restarting the Alpen sequencer before that batch can reach L1 finality
    3. posting a second non-empty batch after restart
    4. asserting the second DA blob appears before the first could finalize
    """

    BATCH_SEALING_BLOCK_COUNT = 10
    L1_REORG_SAFE_DEPTH = 6
    PHASE_BLOCK_WAIT = 25
    PHASE_TX_COUNT = 4
    POLL_ATTEMPTS = 4
    BLOCKS_PER_POLL = 2
    RESTART_PAUSE_SECONDS = 2

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                l1_reorg_safe_depth=self.L1_REORG_SAFE_DEPTH,
                batch_sealing_block_count=self.BATCH_SEALING_BLOCK_COUNT,
            )
        )

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service(ServiceType.Bitcoin)
        sequencer: AlpenClientService = self.runctx.get_service(ServiceType.AlpenSequencer)
        btc_rpc = bitcoin.create_rpc()
        mine_address = btc_rpc.proxy.getnewaddress()
        all_envelopes: list[DaEnvelope] = []
        end_l1 = btc_rpc.proxy.getblockcount()

        eth_rpc = sequencer.create_rpc()
        next_nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)

        # Phase A: create one non-empty batch and observe its DA blob before L1 finality.
        phase_a_deploy_block = self.submit_transfers(eth_rpc, next_nonce, "phase-a")
        next_nonce += self.PHASE_TX_COUNT
        sequencer.advance_to_next_da_window(self.PHASE_BLOCK_WAIT)
        phase_a_blob, end_l1, mined_phase_a = sequencer.wait_for_non_empty_blob(
            btc_rpc,
            mine_address,
            all_envelopes,
            end_l1,
            phase_a_deploy_block,
            "phase-a",
            poll_attempts=self.POLL_ATTEMPTS,
            blocks_per_poll=self.BLOCKS_PER_POLL,
        )

        # Restart while the first blob is still below finality depth.
        logger.info("Restarting Alpen sequencer before first DA blob finalizes...")
        pre_restart_height = sequencer.get_block_number()
        sequencer.stop()
        time.sleep(self.RESTART_PAUSE_SECONDS)
        sequencer.start()
        sequencer.wait_for_ready(timeout=60)
        sequencer.wait_for_block(pre_restart_height + 1, timeout=60)
        eth_rpc = sequencer.create_rpc()

        # Phase B: post another non-empty batch and ensure it reaches L1 before
        # the first blob could possibly finalize.
        phase_b_deploy_block = self.submit_transfers(eth_rpc, next_nonce, "phase-b")
        sequencer.advance_to_next_da_window(self.PHASE_BLOCK_WAIT)
        phase_b_blob, _, mined_phase_b = sequencer.wait_for_non_empty_blob(
            btc_rpc,
            mine_address,
            all_envelopes,
            end_l1,
            phase_b_deploy_block,
            "phase-b",
            poll_attempts=self.POLL_ATTEMPTS,
            blocks_per_poll=self.BLOCKS_PER_POLL,
        )

        total_mined_after_phase_a = mined_phase_a + mined_phase_b
        assert total_mined_after_phase_a < self.L1_REORG_SAFE_DEPTH, (
            "Test mined too many L1 blocks to prove the queue-unblocking case. "
            f"Mined {total_mined_after_phase_a}, "
            f"reorg_safe_depth={self.L1_REORG_SAFE_DEPTH}."
        )
        assert phase_b_blob.last_block_num > phase_a_blob.last_block_num, (
            "Expected the restarted sequencer to post a later non-empty DA blob "
            "before the earlier blob finalized."
        )

        logger.info(
            "Passed: phase A blob last_block_num=%s, phase B blob last_block_num=%s, "
            "total mined after phase A=%s (< finality depth %s)",
            phase_a_blob.last_block_num,
            phase_b_blob.last_block_num,
            total_mined_after_phase_a,
            self.L1_REORG_SAFE_DEPTH,
        )
        return True

    def submit_transfers(self, eth_rpc, start_nonce: int, phase_name: str) -> int:
        """Submit enough transfers to guarantee a non-empty DA batch."""
        recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        deploy_block = int(eth_rpc.eth_blockNumber(), 16)
        logger.info("Submitting %s transfers for %s...", self.PHASE_TX_COUNT, phase_name)
        for offset in range(self.PHASE_TX_COUNT):
            tx_hash = send_eth_transfer(eth_rpc, start_nonce + offset, recipient, 10**18)
            logger.info(
                "  %s tx %s/%s: %s...", phase_name, offset + 1, self.PHASE_TX_COUNT, tx_hash[:20]
            )
        return deploy_block

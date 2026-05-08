"""Verify the new chunked envelope format publishes batches independently.

Under the redesign, reveals do not chain across batches — batch N+1 can be
signed and broadcast without waiting for batch N's commit to confirm. This
test triggers two batches close together and asserts that:

1. At least two distinct commit txs are observed and reassembled.
2. Each commit's reveals spend only that commit's outputs (no
   cross-commit chaining).
3. The scanner reassembles each blob from the reveal chunks funded by its
   own commit tx.

Together these demonstrate that the publishing path no longer serializes
on prior-batch confirmation.
"""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import (
    DaEnvelope,
    reassemble_and_validate_blobs,
    validate_commit_independence,
)
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaParallelPublishingTest(BaseTest):
    """Two batches publish without inter-batch dependency."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                batch_sealing_block_count=3,
            )
        )

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service(ServiceType.Bitcoin)
        sequencer: AlpenClientService = self.runctx.get_service(ServiceType.AlpenSequencer)
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

        # Two waves of transfers separated by enough blocks to land in
        # distinct short test batches.
        logger.info("Sending first wave of transfers")
        for i in range(4):
            send_eth_transfer(eth_rpc, nonce + i, recipient, 10**18)

        # Cross one batch boundary, then trigger the second wave.
        trigger_batch_sealing(sequencer, btc_rpc, num_blocks=8)

        logger.info("Sending second wave of transfers")
        for i in range(4):
            send_eth_transfer(eth_rpc, nonce + 4 + i, recipient, 10**18)

        # Drive enough blocks to seal the second batch and let the lifecycle
        # publish DA for both.
        trigger_batch_sealing(sequencer, btc_rpc, num_blocks=10)

        mine_address = btc_rpc.proxy.getnewaddress()
        all_envelopes: list[DaEnvelope] = []
        commit_count = 0

        for attempt in range(25):
            time.sleep(5)
            btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            # Always re-scan from baseline so commits and their reveals can
            # be paired even when they confirm in different L1 blocks; the
            # scanner is idempotent so we replace the result list each pass.
            end_l1 = btc_rpc.proxy.getblockcount()
            all_envelopes = scan_for_da_envelopes(btc_rpc, baseline_l1_height, end_l1)
            if all_envelopes:
                commit_count = len({e.commit_txid for e in all_envelopes})
                logger.info(
                    "attempt %d: saw %d envelope chunk(s), %d distinct commit(s) so far",
                    attempt + 1,
                    len(all_envelopes),
                    commit_count,
                )
            if commit_count >= 2:
                break

        assert commit_count >= 2, (
            f"expected at least 2 distinct DA commits to demonstrate parallel "
            f"publishing, got {commit_count}"
        )

        # Reveals must not chain off other reveals.
        ok, messages = validate_commit_independence(all_envelopes)
        for m in messages:
            logger.info("  %s", m)
        assert ok, "reveals are chained across commits — independence violated"

        # Each commit's chunks must reassemble to a blob whose hash matches
        results = reassemble_and_validate_blobs(all_envelopes)
        assert len(results) >= 2, f"expected at least 2 reassembled blobs, got {len(results)}"
        for r in results:
            logger.info(
                "blob commit=%s chunks=%d size=%d last_block=%d",
                r.commit_txid,
                r.total_chunks,
                r.total_size,
                r.blob.last_block_num,
            )

        commit_txids = {r.commit_txid for r in results}
        assert len(commit_txids) == len(results), "reassembled blobs unexpectedly share commits"

        return True

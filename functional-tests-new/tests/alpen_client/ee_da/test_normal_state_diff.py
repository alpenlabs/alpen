"""Verify DA is posted for batches with account state changes (ETH transfers)."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import (
    DaEnvelope,
    reassemble_blobs_from_envelopes,
)
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaNormalStateDiffTest(BaseTest):
    """Verify DA is posted for batches with account state changes (ETH transfers)."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                batch_sealing_block_count=30,
            )
        )

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service("bitcoin")
        sequencer: AlpenClientService = self.runctx.get_service("alpen_sequencer")
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

        logger.info("Sending 6 ETH transfers...")
        for i in range(6):
            tx_hash = send_eth_transfer(eth_rpc, nonce + i, recipient, 10**18)
            logger.info(f"  TX {i + 1}/6: {tx_hash[:20]}...")

        # Use a generous block count to ensure the batch containing the
        # transfers is sealed AND the next batch starts (which triggers DA
        # posting for the previous batch).  With batch_sealing_block_count=30,
        # 65 blocks guarantees crossing at least two batch boundaries.
        trigger_batch_sealing(sequencer, btc_rpc, num_blocks=65)

        # Poll for DA envelopes.  After earlier tests the DA lifecycle may
        # need several cycles to catch up through intermediate batches, so
        # use a generous polling window and keep collecting envelopes until
        # we find a non-empty batch.
        mine_address = btc_rpc.proxy.getnewaddress()
        all_envs: list[DaEnvelope] = []
        end_l1 = baseline_l1_height
        non_empty_blob = None

        for attempt in range(20):
            time.sleep(5)
            btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.info(f"Attempt {attempt + 1}: Found {len(new_envs)} DA envelope(s)")
                all_envs.extend(new_envs)

            # Check if we've found a non-empty batch yet
            blobs = reassemble_blobs_from_envelopes(all_envs)
            for blob in blobs:
                if not blob.is_empty_batch():
                    non_empty_blob = blob
                    break

            if non_empty_blob is not None:
                logger.info(f"Found non-empty batch on attempt {attempt + 1}")
                break

            logger.debug(f"Attempt {attempt + 1}: No non-empty batch yet")

        assert non_empty_blob is not None, (
            f"No non-empty DA batch found after {len(all_envs)} envelope(s) collected"
        )
        logger.info(
            f"  DaBlob: last_block_num={non_empty_blob.last_block_num}, "
            f"state_diff={len(non_empty_blob.state_diff)} bytes"
        )

        # Log all blobs for debugging
        blobs = reassemble_blobs_from_envelopes(all_envs)
        for blob in blobs:
            is_empty = blob.is_empty_batch()
            logger.info(
                f"  DaBlob: last_block_num={blob.last_block_num}, "
                f"state_diff={len(blob.state_diff)} bytes, is_empty={is_empty}"
            )

        return True

"""Verify DA is posted even when a batch has no state changes."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import reassemble_blobs_from_envelopes
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaEmptyBatchTest(BaseTest):
    """Verify DA is posted even when a batch has no state changes."""

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
        sequencer: AlpenClientService = self.runctx.get_service("ee_sequencer")
        btc_rpc = bitcoin.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        pre_block = sequencer.get_block_number()
        logger.info(f"Pre-test L2 block: {pre_block}")

        # Seal a batch with no user transactions.
        trigger_batch_sealing(sequencer, btc_rpc)

        # Poll for DA envelopes.
        mine_address = btc_rpc.proxy.getnewaddress()
        envelopes = []
        end_l1 = baseline_l1_height

        for attempt in range(10):
            time.sleep(3)
            btc_rpc.proxy.generatetoaddress(3, mine_address)
            time.sleep(2)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                envelopes.extend(new_envs)
                logger.info(f"Attempt {attempt + 1}: Found {len(new_envs)} DA envelope(s)")
                break
            logger.debug(f"Attempt {attempt + 1}: No envelopes yet")

        assert envelopes, "No DA envelopes found after batch sealing"
        logger.info(f"Found {len(envelopes)} DA envelope(s)")

        blobs = reassemble_blobs_from_envelopes(envelopes)

        empty_batch_found = False
        for blob in blobs:
            is_empty = blob.is_empty_batch()
            logger.info(
                f"  DaBlob: last_block_num={blob.last_block_num}, "
                f"state_diff={len(blob.state_diff)} bytes, is_empty={is_empty}"
            )
            if is_empty:
                empty_batch_found = True
                assert blob.last_block_num > 0, "Empty batch should have valid last_block_num"
                assert len(blob.batch_id_prev_block) == 32, "Empty batch should have valid batch_id"

        assert empty_batch_found, "No empty batch found"
        return True

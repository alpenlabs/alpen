"""Verify DA handles large payloads requiring multiple chunks."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.evm import DEV_ACCOUNT_ADDRESS, deploy_storage_filler
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import (
    DA_CHUNK_HEADER_SIZE,
    DaEnvelope,
    ReassembledBlob,
    reassemble_and_validate_blobs,
    validate_multi_chunk_blob,
    validate_multi_chunk_wtxid_chain,
)
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaMultiChunkTest(BaseTest):
    """Verify DA handles large payloads requiring multiple chunks.

    Deploys many storage-heavy contracts and validates that the resulting
    DA blob is split across multiple chunks with correct reassembly.
    """

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
        sequencer: AlpenClientService = self.runctx.get_service("sequencer")
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)

        # EIP-3860 limits initcode to 49152 bytes.  Each slot needs ~67 bytes of
        # init code (PUSH32 value + PUSH32 key + SSTORE), so max ~700 slots per
        # contract.  Using 500 slots for safety.
        #
        # With batch_sealing_block_count=30, contracts may spread across batches.
        # 80 contracts x 500 slots = 40,000 slots to ensure enough state diff
        # even if split: ~40,000 x 80 bytes = 3.2 MB total.
        num_contracts = 80
        slots_per_contract = 500
        min_expected_chunks = 3

        total_slots = num_contracts * slots_per_contract
        estimated_size_mb = (total_slots * 80) / (1024 * 1024)
        logger.info(
            f"Deploying {num_contracts} contracts with {slots_per_contract} storage slots each..."
        )
        logger.info(
            f"Total slots: {total_slots}, estimated max state diff: ~{estimated_size_mb:.1f} MB"
        )

        pre_deploy_block = sequencer.get_block_number()
        logger.info(f"Current block before deployment: {pre_deploy_block}")

        # Submit ALL transactions without waiting for individual confirmations
        logger.info("Submitting all contract deployments to mempool...")
        tx_hashes = []
        for i in range(num_contracts):
            tx_hash = deploy_storage_filler(eth_rpc, nonce + i, slots_per_contract)
            tx_hashes.append(tx_hash)
        logger.info(f"  Submitted {len(tx_hashes)} transactions to mempool")

        # Wait for ALL transactions to be confirmed
        logger.info("Waiting for all transactions to be confirmed...")
        tx_blocks: dict[str, int] = {}
        start_time = time.time()
        last_logged_count = 0

        while len(tx_blocks) < len(tx_hashes) and (time.time() - start_time) < 180:
            for tx_hash in tx_hashes:
                if tx_hash in tx_blocks:
                    continue
                receipt = eth_rpc.eth_getTransactionReceipt(tx_hash)
                if receipt is not None:
                    tx_blocks[tx_hash] = int(receipt["blockNumber"], 16)

            confirmed = len(tx_blocks)
            if confirmed > last_logged_count and confirmed % 10 == 0:
                last_logged_count = confirmed
                blocks_used = set(tx_blocks.values())
                logger.info(
                    f"  Confirmed {confirmed}/{len(tx_hashes)} txs"
                    f" across blocks: {sorted(blocks_used)}"
                )
            time.sleep(0.5)

        if len(tx_blocks) < len(tx_hashes):
            missing = len(tx_hashes) - len(tx_blocks)
            raise AssertionError(f"{missing} contract deployments not confirmed within timeout")

        # Analyze block distribution
        blocks_used = sorted(set(tx_blocks.values()))
        max_contract_block = max(blocks_used)
        logger.info(
            f"All {len(tx_hashes)} contracts deployed across blocks"
            f" {min(blocks_used)} to {max_contract_block}"
        )

        contracts_per_block: dict[int, int] = {}
        for block in tx_blocks.values():
            contracts_per_block[block] = contracts_per_block.get(block, 0) + 1
        for block in sorted(contracts_per_block.keys()):
            count = contracts_per_block[block]
            slots_in_block = count * slots_per_contract
            estimated_diff_kb = (slots_in_block * 80) / 1024
            logger.info(
                f"  Block {block}: {count} contracts,"
                f" ~{slots_in_block} slots,"
                f" ~{estimated_diff_kb:.0f} KB state diff"
            )

        # The DA environment uses batch_sealing_block_count=30
        batch_sealing_block_count = 30
        expected_batch_last_block = (
            (max_contract_block - 1) // batch_sealing_block_count + 1
        ) * batch_sealing_block_count
        logger.info(f"Expecting contracts in batch ending at block {expected_batch_last_block}")

        # Poll for the multi-chunk blob
        all_envelopes: list[DaEnvelope] = []
        multi_chunk_result: ReassembledBlob | None = None
        end_l1 = baseline_l1_height
        mine_address = btc_rpc.proxy.getnewaddress()

        for attempt in range(30):
            current_l2_block = sequencer.get_block_number()
            blocks_needed = expected_batch_last_block + batch_sealing_block_count
            if current_l2_block < blocks_needed:
                logger.debug(
                    f"Attempt {attempt + 1}: Waiting for L2 block"
                    f" {blocks_needed} (current: {current_l2_block})"
                )
                sequencer.wait_for_block(blocks_needed, timeout=300)

            logger.debug(f"Attempt {attempt + 1}: Waiting for DA transactions to reach mempool...")
            time.sleep(10)

            mempool_info = btc_rpc.proxy.getmempoolinfo()
            logger.debug(
                f"Attempt {attempt + 1}: Mempool has {mempool_info.get('size', 0)} transaction(s)"
            )

            btc_rpc.proxy.generatetoaddress(10, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envelopes = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)

            if new_envelopes:
                logger.info(f"Attempt {attempt + 1}: Found {len(new_envelopes)} new DA envelope(s)")
                for env in new_envelopes:
                    chunk_size = len(env.payload) - DA_CHUNK_HEADER_SIZE
                    logger.debug(
                        f"  Chunk {env.chunk_index}/{env.total_chunks}: {chunk_size} bytes, "
                        f"blob_hash={env.blob_hash.hex()[:16]}..."
                    )

                all_envelopes.extend(new_envelopes)

                results = reassemble_and_validate_blobs(all_envelopes)
                for result in results:
                    logger.debug(
                        f"  Reassembled blob: last_block_num={result.blob.last_block_num}, "
                        f"total_chunks={result.total_chunks}, total_size={result.total_size} bytes"
                    )
                    if result.total_chunks >= min_expected_chunks:
                        multi_chunk_result = result
                        logger.info(f"  Found multi-chunk blob with {result.total_chunks} chunks!")
            else:
                logger.debug(f"Attempt {attempt + 1}: No new envelopes found")

            if multi_chunk_result is not None:
                break

        assert multi_chunk_result is not None, (
            f"Expected multi-chunk blob with at least {min_expected_chunks} chunks. "
            f"Contracts deployed in blocks up to {max_contract_block}. "
            f"Expected batch ending at block {expected_batch_last_block}. "
            f"Total envelopes collected: {len(all_envelopes)}"
        )

        logger.info("Multi-chunk blob validation:")
        is_valid, messages = validate_multi_chunk_blob(
            multi_chunk_result, min_chunks=min_expected_chunks
        )
        for msg in messages:
            logger.info(f"  {msg}")
        assert is_valid, "Multi-chunk validation failed"

        logger.info("Multi-chunk wtxid chain validation:")
        wtxid_valid, wtxid_messages = validate_multi_chunk_wtxid_chain(
            all_envelopes,
            multi_chunk_result.blob_hash,
        )
        for msg in wtxid_messages:
            logger.info(f"  {msg}")
        assert wtxid_valid, "Wtxid chain validation failed within multi-chunk blob"

        logger.info(
            f"Passed: {multi_chunk_result.total_chunks} chunks, "
            f"{multi_chunk_result.total_size} bytes total"
        )
        return True

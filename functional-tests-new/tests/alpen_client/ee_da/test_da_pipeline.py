"""
Tests the EE data availability pipeline.

This test validates the complete DA lifecycle through multiple scenarios:

1. Normal State Diff — ETH transfers create account balance changes
2. Empty Batch — blocks with no transactions still produce DA
3. Large State Diff (Multi-Chunk) — contract deployments trigger chunked DA
4. Cross-Batch Bytecode Dedup — repeated bytecodes are filtered from later blobs

Additionally validates:
- Wtxid chain: each DA envelope references the previous envelope's tail wtxid
- Batch progression: last_block_num increases monotonically
"""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.services import AlpenClientService, BitcoinService
from tests.alpen_client.ee_da.codec import (
    DA_CHUNK_HEADER_SIZE,
    DaEnvelope,
    ReassembledBlob,
    reassemble_and_validate_blobs,
    reassemble_blobs_from_envelopes,
    validate_multi_chunk_blob,
    validate_multi_chunk_wtxid_chain,
    validate_wtxid_chain,
)
from tests.alpen_client.ee_da.evm import (
    DEV_ACCOUNT_ADDRESS,
    deploy_large_runtime_contract,
    deploy_storage_filler,
    send_eth_transfer,
)
from tests.alpen_client.ee_da.helpers import (
    TestState,
    scan_for_da_envelopes,
    trigger_batch_sealing,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaPipeline(BaseTest):
    """
    End-to-end test of the EE data availability pipeline.

    Runs multiple scenarios sequentially to validate different DA cases.
    Uses TestState to track blobs across scenarios and validate:
    - last_block_num progression
    - Wtxid chain integrity
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_da")

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service("bitcoin")
        self._sequencer: AlpenClientService = self.runctx.get_service("sequencer")
        self._btc_rpc = bitcoin.create_rpc()
        self._eth_rpc = self._sequencer.create_rpc()

        state = TestState()
        state.baseline_l1_height = self._btc_rpc.proxy.getblockcount()

        l1_cursor = state.baseline_l1_height
        l1_cursor = self._scenario_normal_state_diff(state, l1_cursor)
        l1_cursor = self._scenario_empty_batch(state, l1_cursor)
        l1_cursor = self._scenario_multi_chunk(state, l1_cursor)
        self._scenario_bytecode_dedup(state, l1_cursor)

        self._final_validation(state)
        return True

    # -----------------------------------------------------------------
    # Scenario 1: Normal State Diff
    # -----------------------------------------------------------------

    def _scenario_normal_state_diff(self, state: TestState, l1_cursor: int) -> int:
        """Verify DA is posted for batches with account state changes."""
        logger.info("=" * 70)
        logger.info("SCENARIO 1: Normal State Diff (ETH Transfers)")
        logger.info("=" * 70)

        nonce = int(self._eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

        logger.info("Sending 6 ETH transfers...")
        for i in range(6):
            tx_hash = send_eth_transfer(self._eth_rpc, nonce + i, recipient, 10**18)
            logger.info(f"  TX {i + 1}/6: {tx_hash[:20]}...")

        trigger_batch_sealing(self._sequencer, self._btc_rpc)

        # Scan for DA envelopes (start after baseline — no DA before test activity)
        end_l1 = self._btc_rpc.proxy.getblockcount()
        new_envelopes = scan_for_da_envelopes(self._btc_rpc, l1_cursor + 1, end_l1)

        assert new_envelopes, "SCENARIO 1 FAILED: No DA envelopes found"
        logger.info(f"Found {len(new_envelopes)} DA envelope(s)")

        assert validate_wtxid_chain(new_envelopes), "SCENARIO 1 FAILED: Wtxid chain broken"
        logger.info("Wtxid chain validation passed")

        state.envelopes.extend(new_envelopes)

        blobs = reassemble_blobs_from_envelopes(new_envelopes)
        assert blobs, "SCENARIO 1 FAILED: Could not reassemble any DA blobs"

        non_empty_found = False
        for blob in blobs:
            logger.info(
                f"  DaBlob: last_block_num={blob.last_block_num}, "
                f"state_diff={len(blob.state_diff)} bytes"
            )
            state.blobs.append(blob)
            state.max_block_num = max(state.max_block_num, blob.last_block_num)
            if not blob.is_empty_batch():
                non_empty_found = True

        assert non_empty_found, "SCENARIO 1 FAILED: No non-empty batch found"
        logger.info(f"SCENARIO 1 PASSED: max_block_num={state.max_block_num}")
        return end_l1

    # -----------------------------------------------------------------
    # Scenario 2: Empty Batch
    # -----------------------------------------------------------------

    def _scenario_empty_batch(self, state: TestState, l1_cursor: int) -> int:
        """Verify DA is posted even when batch has no state changes."""
        logger.info("")
        logger.info("=" * 70)
        logger.info("SCENARIO 2: Empty Batch (No Transactions)")
        logger.info("=" * 70)

        prev_max_block = state.max_block_num
        logger.info(f"Looking for blobs with last_block_num > {prev_max_block}")

        logger.info("Waiting for blocks without transactions...")
        trigger_batch_sealing(self._sequencer, self._btc_rpc)

        end_l1 = self._btc_rpc.proxy.getblockcount()
        new_envelopes = scan_for_da_envelopes(self._btc_rpc, l1_cursor + 1, end_l1)

        assert new_envelopes, "SCENARIO 2 FAILED: No DA envelopes found after batch sealing"
        logger.info(f"Found {len(new_envelopes)} new DA envelope(s)")
        state.envelopes.extend(new_envelopes)

        new_blobs = reassemble_blobs_from_envelopes(new_envelopes)
        scenario_blobs = [b for b in new_blobs if b.last_block_num > prev_max_block]
        logger.info(f"Found {len(scenario_blobs)} blob(s) from SCENARIO 2 period")

        empty_batch_found = False
        for blob in scenario_blobs:
            is_empty = blob.is_empty_batch()
            logger.info(
                f"  DaBlob: last_block_num={blob.last_block_num}, "
                f"state_diff={len(blob.state_diff)} bytes, is_empty={is_empty}"
            )
            state.blobs.append(blob)
            state.max_block_num = max(state.max_block_num, blob.last_block_num)

            if is_empty:
                empty_batch_found = True
                assert blob.last_block_num > 0, "Empty batch should have valid last_block_num"
                assert len(blob.batch_id_prev_block) == 32, "Empty batch should have valid batch_id"

        assert empty_batch_found, "SCENARIO 2 FAILED: No empty batch found in new blobs"
        logger.info(f"SCENARIO 2 PASSED: max_block_num={state.max_block_num}")
        return end_l1

    # -----------------------------------------------------------------
    # Scenario 3: Multi-Chunk DA (Large State Diff)
    # -----------------------------------------------------------------

    def _scenario_multi_chunk(self, state: TestState, l1_cursor: int) -> int:
        """Verify DA handles large payloads requiring multiple chunks.

        Deploys many storage-heavy contracts and validates that the resulting
        DA blob is split across multiple chunks with correct reassembly.
        """
        logger.info("")
        logger.info("=" * 70)
        logger.info("SCENARIO 3: Multi-Chunk DA (Large State Diff)")
        logger.info("=" * 70)

        nonce = int(self._eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)

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
        logger.info(f"Target: at least {min_expected_chunks} chunks (validates multi-chunk DA)")

        pre_deploy_block = self._sequencer.get_block_number()
        logger.info(f"Current block before deployment: {pre_deploy_block}")

        # Submit ALL transactions without waiting for individual confirmations
        logger.info("Submitting all contract deployments to mempool...")
        tx_hashes = []
        for i in range(num_contracts):
            tx_hash = deploy_storage_filler(self._eth_rpc, nonce + i, slots_per_contract)
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
                receipt = self._eth_rpc.eth_getTransactionReceipt(tx_hash)
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
        logger.info(f"  Blocks used: {blocks_used}")

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
        multi_chunk_envelopes: list[DaEnvelope] = []
        multi_chunk_result: ReassembledBlob | None = None
        end_l1 = l1_cursor
        mine_address = self._btc_rpc.proxy.getnewaddress()

        for attempt in range(30):
            current_l2_block = self._sequencer.get_block_number()
            blocks_needed = expected_batch_last_block + batch_sealing_block_count
            if current_l2_block < blocks_needed:
                logger.info(
                    f"Attempt {attempt + 1}: Waiting for L2 block"
                    f" {blocks_needed} (current: {current_l2_block})"
                )
                self._sequencer.wait_for_block(blocks_needed, timeout=120)

            logger.info(f"Attempt {attempt + 1}: Waiting for DA transactions to reach mempool...")
            time.sleep(10)

            mempool_info = self._btc_rpc.proxy.getmempoolinfo()
            logger.info(
                f"Attempt {attempt + 1}: Mempool has {mempool_info.get('size', 0)} transaction(s)"
            )

            self._btc_rpc.proxy.generatetoaddress(10, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = self._btc_rpc.proxy.getblockcount()
            new_envelopes = scan_for_da_envelopes(self._btc_rpc, prev_end + 1, end_l1)

            if new_envelopes:
                logger.info(f"Attempt {attempt + 1}: Found {len(new_envelopes)} new DA envelope(s)")
                for env in new_envelopes:
                    chunk_size = len(env.payload) - DA_CHUNK_HEADER_SIZE
                    logger.info(
                        f"  Chunk {env.chunk_index}/{env.total_chunks}: {chunk_size} bytes, "
                        f"blob_hash={env.blob_hash.hex()[:16]}..."
                    )

                multi_chunk_envelopes.extend(new_envelopes)
                state.envelopes.extend(new_envelopes)

                results = reassemble_and_validate_blobs(multi_chunk_envelopes)
                for result in results:
                    logger.info(
                        f"  Reassembled blob: last_block_num={result.blob.last_block_num}, "
                        f"total_chunks={result.total_chunks}, total_size={result.total_size} bytes"
                    )
                    state.blobs.append(result.blob)
                    state.max_block_num = max(state.max_block_num, result.blob.last_block_num)

                    if result.total_chunks >= min_expected_chunks:
                        multi_chunk_result = result
                        logger.info(f"  Found multi-chunk blob with {result.total_chunks} chunks!")
            else:
                logger.info(f"Attempt {attempt + 1}: No new envelopes found")

            if multi_chunk_result is not None:
                break

        assert multi_chunk_result is not None, (
            "SCENARIO 3 FAILED: Expected multi-chunk blob with at least"
            f" {min_expected_chunks} chunks. "
            f"Contracts deployed in blocks up to {max_contract_block}. "
            f"Expected batch ending at block {expected_batch_last_block}. "
            f"Total envelopes collected: {len(multi_chunk_envelopes)}"
        )

        logger.info("")
        logger.info("Multi-chunk blob validation:")
        is_valid, messages = validate_multi_chunk_blob(
            multi_chunk_result, min_chunks=min_expected_chunks
        )
        for msg in messages:
            logger.info(f"  {msg}")
        assert is_valid, "SCENARIO 3 FAILED: Multi-chunk validation failed"

        logger.info("")
        logger.info("Multi-chunk wtxid chain validation:")
        wtxid_valid, wtxid_messages = validate_multi_chunk_wtxid_chain(
            multi_chunk_envelopes,
            multi_chunk_result.blob_hash,
        )
        for msg in wtxid_messages:
            logger.info(f"  {msg}")
        assert wtxid_valid, (
            "SCENARIO 3 FAILED: Wtxid chain validation failed within multi-chunk blob"
        )

        logger.info(
            f"SCENARIO 3 PASSED: {multi_chunk_result.total_chunks} chunks, "
            f"{multi_chunk_result.total_size} bytes total"
        )
        return end_l1

    # -----------------------------------------------------------------
    # Scenario 4: Cross-Batch Bytecode Deduplication
    # -----------------------------------------------------------------

    def _scenario_bytecode_dedup(self, state: TestState, l1_cursor: int) -> int:
        """Verify duplicate bytecodes are filtered from later DA blobs.

        Phase A deploys contracts with a large runtime bytecode. After DA
        finalization (code hashes marked as published), Phase B deploys the
        same bytecode and asserts the DA blob is significantly smaller.
        """
        logger.info("")
        logger.info("=" * 70)
        logger.info("SCENARIO 4: Cross-Batch Bytecode Deduplication")
        logger.info("=" * 70)

        dedup_runtime_size = 10_000  # 10 KB deterministic runtime bytecode
        mine_address = self._btc_rpc.proxy.getnewaddress()

        # --- Phase A: Deploy contracts with a large unique runtime bytecode ---
        logger.info(
            f"Phase A: Deploying 3 contracts with {dedup_runtime_size}-byte "
            f"runtime bytecode (first occurrence)"
        )

        nonce = int(self._eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        phase_a_deploy_block = self._sequencer.get_block_number()
        for i in range(3):
            tx_hash = deploy_large_runtime_contract(self._eth_rpc, nonce + i, dedup_runtime_size)
            logger.info(f"  Phase A contract {i + 1}/3: {tx_hash[:20]}...")
        logger.info(f"  Deployed at L2 block ~{phase_a_deploy_block}")

        trigger_batch_sealing(self._sequencer, self._btc_rpc)

        # Poll for Phase A DA blob
        phase_a_blob = None
        phase_a_all_envs: list[DaEnvelope] = []
        end_l1 = l1_cursor

        for attempt in range(20):
            time.sleep(5)
            self._btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = self._btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(self._btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.info(f"  Phase A attempt {attempt + 1}: Found {len(new_envs)} envelope(s)")
                phase_a_all_envs.extend(new_envs)
                state.envelopes.extend(new_envs)

            blobs = reassemble_blobs_from_envelopes(phase_a_all_envs)
            for b in blobs:
                if b.last_block_num > phase_a_deploy_block and not b.is_empty_batch():
                    state.blobs.append(b)
                    state.max_block_num = max(state.max_block_num, b.last_block_num)
                    if phase_a_blob is None or len(b.state_diff) > len(phase_a_blob.state_diff):
                        phase_a_blob = b

            if phase_a_blob is not None:
                logger.info(f"  Found Phase A blob on attempt {attempt + 1}")
                break

        assert phase_a_blob is not None, (
            "SCENARIO 4 Phase A FAILED: No DA blob found containing "
            f"Phase A contracts (deployed at L2 block ~{phase_a_deploy_block})"
        )
        phase_a_diff_size = len(phase_a_blob.state_diff)
        logger.info(
            f"Phase A blob: last_block_num={phase_a_blob.last_block_num}, "
            f"state_diff={phase_a_diff_size} bytes"
        )

        # --- Wait for DA finalization + lifecycle code-hash marking ---
        logger.info("Waiting for DA finalization and code hash marking...")
        for _ in range(10):
            self._btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

        # --- Phase B: Deploy contracts with the SAME runtime bytecode ---
        logger.info(
            f"Phase B: Deploying 3 contracts with same {dedup_runtime_size}-byte "
            f"runtime bytecode (should be deduplicated)"
        )

        nonce = int(self._eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        phase_b_deploy_block = self._sequencer.get_block_number()
        for i in range(3):
            tx_hash = deploy_large_runtime_contract(self._eth_rpc, nonce + i, dedup_runtime_size)
            logger.info(f"  Phase B contract {i + 1}/3: {tx_hash[:20]}...")
        logger.info(f"  Deployed at L2 block ~{phase_b_deploy_block}")

        trigger_batch_sealing(self._sequencer, self._btc_rpc)

        # Poll for Phase B DA blob
        phase_b_blob = None
        phase_b_all_envs: list[DaEnvelope] = []

        for attempt in range(20):
            time.sleep(5)
            self._btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = self._btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(self._btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.info(f"  Phase B attempt {attempt + 1}: Found {len(new_envs)} envelope(s)")
                phase_b_all_envs.extend(new_envs)
                state.envelopes.extend(new_envs)

            blobs = reassemble_blobs_from_envelopes(phase_b_all_envs)
            for b in blobs:
                if b.last_block_num > phase_b_deploy_block and not b.is_empty_batch():
                    state.blobs.append(b)
                    state.max_block_num = max(state.max_block_num, b.last_block_num)
                    if phase_b_blob is None or len(b.state_diff) > len(phase_b_blob.state_diff):
                        phase_b_blob = b

            if phase_b_blob is not None:
                logger.info(f"  Found Phase B blob on attempt {attempt + 1}")
                break

        assert phase_b_blob is not None, (
            "SCENARIO 4 Phase B FAILED: No DA blob found containing "
            f"Phase B contracts (deployed at L2 block ~{phase_b_deploy_block})"
        )
        phase_b_diff_size = len(phase_b_blob.state_diff)
        logger.info(
            f"Phase B blob: last_block_num={phase_b_blob.last_block_num}, "
            f"state_diff={phase_b_diff_size} bytes"
        )

        # --- Validate bytecode deduplication ---
        size_reduction = phase_a_diff_size - phase_b_diff_size
        min_expected_savings = int(dedup_runtime_size * 0.8)

        logger.info("Bytecode deduplication results:")
        logger.info(f"  Phase A state_diff (with bytecode):  {phase_a_diff_size} bytes")
        logger.info(f"  Phase B state_diff (deduped):        {phase_b_diff_size} bytes")
        logger.info(f"  Size reduction:                      {size_reduction} bytes")
        logger.info(f"  Minimum expected savings:            {min_expected_savings} bytes")

        assert size_reduction >= min_expected_savings, (
            f"SCENARIO 4 FAILED: Bytecode deduplication did not save enough space. "
            f"Phase A: {phase_a_diff_size} bytes, Phase B: {phase_b_diff_size} bytes, "
            f"reduction: {size_reduction} bytes, expected >= {min_expected_savings} bytes. "
            f"The {dedup_runtime_size}-byte runtime bytecode should have been "
            f"filtered from Phase B's DA blob."
        )

        logger.info(
            f"SCENARIO 4 PASSED: Bytecode deduplication saved {size_reduction} bytes "
            f"({size_reduction * 100 // dedup_runtime_size}% of runtime size)"
        )
        return end_l1

    # -----------------------------------------------------------------
    # Final Validation
    # -----------------------------------------------------------------

    def _final_validation(self, state: TestState) -> None:
        """Validate wtxid chain and block progression across all scenarios."""
        logger.info("")
        logger.info("=" * 70)
        logger.info("FINAL VALIDATION: Wtxid Chain Integrity")
        logger.info("=" * 70)

        logger.info(f"Total envelopes: {len(state.envelopes)}")
        logger.info(f"Total blobs: {len(state.blobs)}")

        assert validate_wtxid_chain(state.envelopes), (
            "FINAL VALIDATION FAILED: Wtxid chain broken across scenarios"
        )
        logger.info("Wtxid chain validation passed")

        block_nums = sorted(b.last_block_num for b in state.blobs)
        logger.info(f"Block number progression: {block_nums}")

        logger.info("")
        logger.info("=" * 70)
        logger.info("ALL SCENARIOS PASSED - DA Pipeline validated successfully")
        logger.info("=" * 70)

"""Verify duplicate bytecodes are filtered from later DA blobs."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.evm import DEV_ACCOUNT_ADDRESS, deploy_large_runtime_contract
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import DaEnvelope, reassemble_blobs_from_envelopes
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaBytecodeDeduplicationTest(BaseTest):
    """Verify duplicate bytecodes are filtered from later DA blobs.

    Phase A deploys contracts with a large runtime bytecode. After DA
    finalization (code hashes marked as published), Phase B deploys the
    same bytecode and asserts the DA blob is significantly smaller.
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
        sequencer: AlpenClientService = self.runctx.get_service("ee_sequencer")
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        dedup_runtime_size = 10_000  # 10 KB deterministic runtime bytecode
        mine_address = btc_rpc.proxy.getnewaddress()

        # --- Phase A: Deploy contracts with a large unique runtime bytecode ---
        logger.info(
            f"Phase A: Deploying 3 contracts with {dedup_runtime_size}-byte "
            f"runtime bytecode (first occurrence)"
        )

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        phase_a_deploy_block = sequencer.get_block_number()
        for i in range(3):
            tx_hash = deploy_large_runtime_contract(eth_rpc, nonce + i, dedup_runtime_size)
            logger.info(f"  Phase A contract {i + 1}/3: {tx_hash[:20]}...")

        trigger_batch_sealing(sequencer, btc_rpc)

        # Poll for Phase A DA blob
        phase_a_blob = None
        phase_a_all_envs: list[DaEnvelope] = []
        end_l1 = baseline_l1_height

        for attempt in range(20):
            time.sleep(5)
            btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.debug(f"  Phase A attempt {attempt + 1}: Found {len(new_envs)} envelope(s)")
                phase_a_all_envs.extend(new_envs)

            blobs = reassemble_blobs_from_envelopes(phase_a_all_envs)
            for b in blobs:
                if b.last_block_num > phase_a_deploy_block and not b.is_empty_batch():
                    if phase_a_blob is None or len(b.state_diff) > len(phase_a_blob.state_diff):
                        phase_a_blob = b

            if phase_a_blob is not None:
                logger.info(f"  Found Phase A blob on attempt {attempt + 1}")
                break

        assert phase_a_blob is not None, (
            "Phase A: No DA blob found containing contracts "
            f"(deployed at L2 block ~{phase_a_deploy_block})"
        )
        phase_a_diff_size = len(phase_a_blob.state_diff)
        logger.info(
            f"Phase A blob: last_block_num={phase_a_blob.last_block_num}, "
            f"state_diff={phase_a_diff_size} bytes"
        )

        # --- Wait for DA finalization + lifecycle code-hash marking ---
        logger.info("Waiting for DA finalization and code hash marking...")
        for _ in range(10):
            btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

        # --- Phase B: Deploy contracts with the SAME runtime bytecode ---
        logger.info(
            f"Phase B: Deploying 3 contracts with same {dedup_runtime_size}-byte "
            f"runtime bytecode (should be deduplicated)"
        )

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        phase_b_deploy_block = sequencer.get_block_number()
        for i in range(3):
            tx_hash = deploy_large_runtime_contract(eth_rpc, nonce + i, dedup_runtime_size)
            logger.info(f"  Phase B contract {i + 1}/3: {tx_hash[:20]}...")

        trigger_batch_sealing(sequencer, btc_rpc)

        # Poll for Phase B DA blob
        phase_b_blob = None
        phase_b_all_envs: list[DaEnvelope] = []

        for attempt in range(20):
            time.sleep(5)
            btc_rpc.proxy.generatetoaddress(5, mine_address)
            time.sleep(3)

            prev_end = end_l1
            end_l1 = btc_rpc.proxy.getblockcount()
            new_envs = scan_for_da_envelopes(btc_rpc, prev_end + 1, end_l1)
            if new_envs:
                logger.debug(f"  Phase B attempt {attempt + 1}: Found {len(new_envs)} envelope(s)")
                phase_b_all_envs.extend(new_envs)

            blobs = reassemble_blobs_from_envelopes(phase_b_all_envs)
            for b in blobs:
                if b.last_block_num > phase_b_deploy_block and not b.is_empty_batch():
                    if phase_b_blob is None or len(b.state_diff) > len(phase_b_blob.state_diff):
                        phase_b_blob = b

            if phase_b_blob is not None:
                logger.info(f"  Found Phase B blob on attempt {attempt + 1}")
                break

        assert phase_b_blob is not None, (
            "Phase B: No DA blob found containing contracts "
            f"(deployed at L2 block ~{phase_b_deploy_block})"
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
            f"Bytecode deduplication did not save enough space. "
            f"Phase A: {phase_a_diff_size} bytes, Phase B: {phase_b_diff_size} bytes, "
            f"reduction: {size_reduction} bytes, expected >= {min_expected_savings} bytes."
        )

        logger.info(
            f"Passed: Bytecode deduplication saved {size_reduction} bytes "
            f"({size_reduction * 100 // dedup_runtime_size}% of runtime size)"
        )
        return True

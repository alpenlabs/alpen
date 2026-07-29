"""Verify EE DA bytes on L1 match local encoding and replay to the same state root."""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.services import AlpenClientService, BitcoinService
from common.wait import timeout_for_expected_blocks, wait_until, wait_until_with_value
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import (
    DaEnvelope,
    reassemble_raw_blobs_from_envelopes,
)
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing
from tests.dbtool.helpers import run_dbtool_ee_json

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaPublicationParityReconstruction(BaseTest):
    """Verify published EE DA bytes reconstruct the authenticated EVM state root."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                batch_sealing_block_count=3,
            )
        )

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service("bitcoin")
        sequencer: AlpenClientService = self.runctx.get_service(ServiceType.AlpenSequencer)
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        baseline_l1_height = btc_rpc.proxy.getblockcount()

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

        logger.info("Sending ETH transfers for DA publication parity test...")
        tx_hashes = [send_eth_transfer(eth_rpc, nonce + i, recipient, 10**18) for i in range(6)]

        trigger_batch_sealing(sequencer, btc_rpc, num_blocks=10)

        tx_blocks: dict[str, int] = {}

        def all_transfers_confirmed():
            for tx_hash in tx_hashes:
                if tx_hash in tx_blocks:
                    continue
                receipt = eth_rpc.eth_getTransactionReceipt(tx_hash)
                if receipt is None:
                    return False
                assert int(receipt.get("status", "0x1"), 16) == 1, (
                    f"transfer {tx_hash} failed with receipt {receipt}"
                )
                tx_blocks[tx_hash] = int(receipt["blockNumber"], 16)
            return len(tx_blocks) == len(tx_hashes)

        wait_until(
            all_transfers_confirmed,
            error_with="ETH transfers were not confirmed before DA polling",
            timeout=timeout_for_expected_blocks(10, seconds_per_block=15.0, slack_seconds=60),
            step=0.5,
        )

        target_block_num = max(tx_blocks.values())

        mine_address = btc_rpc.proxy.getnewaddress()

        def poll_for_target_blob():
            btc_rpc.proxy.generatetoaddress(5, mine_address)

            end_l1 = btc_rpc.proxy.getblockcount()
            all_envs: list[DaEnvelope] = scan_for_da_envelopes(btc_rpc, baseline_l1_height, end_l1)
            if all_envs:
                logger.info("Saw %s DA envelope chunk(s)", len(all_envs))

            raw_blobs = reassemble_raw_blobs_from_envelopes(all_envs)
            # dbtool selects the earliest blob whose last_block_num covers the target.
            # Because the target is a transfer block, that earliest covering blob is
            # non-empty; the filter only skips later empty batches while polling.
            candidates = [
                (commit_txid, raw_blob, blob)
                for commit_txid, raw_blob, blob in raw_blobs
                if blob.last_block_num >= target_block_num and not blob.is_empty_batch()
            ]
            if candidates:
                target_l1_blob = min(candidates, key=lambda item: item[2].last_block_num)
                logger.info(
                    "Found target DA blob: last_block_num=%s raw_bytes=%s",
                    target_l1_blob[2].last_block_num,
                    len(target_l1_blob[1]),
                )
                return target_l1_blob

            logger.debug("No target DA blob yet")
            return None

        target_l1_blob = wait_until_with_value(
            poll_for_target_blob,
            predicate=lambda v: v is not None,
            error_with=f"No non-empty DA blob covering EVM block {target_block_num} found",
            timeout=160,
            step=5,
        )

        commit_txid, l1_blob_bytes, l1_blob = target_l1_blob
        batch_block = eth_rpc.eth_getBlockByNumber(hex(l1_blob.last_block_num), False)
        assert batch_block is not None, f"missing batch EVM block {l1_blob.last_block_num}"
        expected_state_root = batch_block["stateRoot"].lower()

        # dbtool opens the same sled directory, so the alpen-client must be stopped first.
        ee_datadir = sequencer.props["datadir"]
        sequencer.stop()

        dbtool = run_dbtool_ee_json(
            ee_datadir,
            "ee-da-inspect",
            "--chain",
            "dev",
            "--target-last-block",
            str(target_block_num),
            timeout=120,
        )

        assert dbtool["target"]["last_block_num"] == l1_blob.last_block_num, (
            dbtool,
            commit_txid,
        )
        assert dbtool["target"]["local_blob_hex"] == l1_blob_bytes.hex(), (
            "L1-reassembled DA bytes must match producer-local encoded bytes"
        )
        assert dbtool["replay"]["post_state_root"].lower() == expected_state_root, (
            dbtool,
            expected_state_root,
        )

        return True

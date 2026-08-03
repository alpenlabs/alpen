"""Verify canonical EE DA scanning drops envelopes from invalidated L1 blocks."""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services import BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import reassemble_and_validate_blobs
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes
from tests.alpen_client.ee_da.injection import (
    broadcast_raw_tx,
    mine_blocks,
    mine_empty_blocks,
    post_ee_da_envelope,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaReorgInvalidationTest(BaseTest):
    """Invalidate DA-containing L1 blocks and scan only the canonical chain."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
            )
        )

    def main(self, ctx) -> bool:
        bitcoin: BitcoinService = self.runctx.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        start_height = btc_rpc.proxy.getblockcount() + 1
        invalidated = post_ee_da_envelope(
            bitcoin,
            chunks=[_make_da_blob(11, 101, b"will be reorged")],
            reveal_count=0,
            mine_mode="manual",
        )

        commit_block_hash = mine_blocks(bitcoin, 1)[0]
        broadcast_raw_tx(bitcoin, invalidated.reveal_txs[0].hex)
        reveal_block_hash = mine_blocks(bitcoin, 1)[0]
        old_tip = btc_rpc.proxy.getblockcount()

        old_envelopes = scan_for_da_envelopes(btc_rpc, start_height, old_tip)
        assert any(env.commit_txid == invalidated.commit_txid for env in old_envelopes), (
            "sanity check failed: original canonical window did not include injected DA"
        )

        logger.info(
            "invalidating DA blocks commit_block=%s reveal_block=%s",
            commit_block_hash,
            reveal_block_hash,
        )
        btc_rpc.proxy.invalidateblock(commit_block_hash)
        regressed_tip = btc_rpc.proxy.getblockcount()
        assert regressed_tip < start_height, (
            f"expected tip below DA window after invalidation, got {regressed_tip}"
        )

        _deprioritize_if_in_mempool(
            btc_rpc,
            [
                invalidated.commit_txid,
                invalidated.reveal_txs[0].txid,
            ],
        )
        mine_empty_blocks(bitcoin, 4)
        replacement_tip = btc_rpc.proxy.getblockcount()

        replacement_envelopes = scan_for_da_envelopes(btc_rpc, start_height, replacement_tip)
        invalidated_txids = {invalidated.commit_txid, invalidated.reveal_txs[0].txid}
        observed_txids = {
            txid for env in replacement_envelopes for txid in (env.commit_txid, env.reveal_txid)
        }
        leaked_txids = invalidated_txids & observed_txids
        assert invalidated_txids.isdisjoint(observed_txids), (
            f"invalidated DA txids still appear in canonical scan: {leaked_txids}"
        )

        honest_start = btc_rpc.proxy.getblockcount() + 1
        honest = post_ee_da_envelope(
            bitcoin,
            chunks=[_make_da_blob(12, 102, b"replacement honest")],
        )
        final_tip = btc_rpc.proxy.getblockcount()
        final_envelopes = scan_for_da_envelopes(btc_rpc, start_height, final_tip)
        final_txids = {
            txid for env in final_envelopes for txid in (env.commit_txid, env.reveal_txid)
        }
        assert invalidated_txids.isdisjoint(final_txids), (
            "invalidated DA txids reappeared after later honest publication"
        )

        honest_envelopes = scan_for_da_envelopes(btc_rpc, honest_start, final_tip)
        results = reassemble_and_validate_blobs(honest_envelopes)
        assert len(results) == 1, f"expected one honest blob after reorg, got {len(results)}"
        assert results[0].commit_txid == honest.commit_txid
        assert results[0].blob.last_block_num == 102
        assert results[0].blob.state_diff == b"replacement honest"

        return True


def _deprioritize_if_in_mempool(btc_rpc, txids: list[str]) -> None:
    mempool = set(btc_rpc.proxy.getrawmempool())
    for txid in txids:
        if txid not in mempool:
            continue
        btc_rpc.proxy.prioritisetransaction(txid, 0, -100_000_000)


def _make_da_blob(update_seq_no: int, block_num: int, state_diff: bytes) -> bytes:
    return b"".join(
        [
            update_seq_no.to_bytes(8, "big"),
            block_num.to_bytes(8, "big"),
            (1_700_000_000).to_bytes(8, "big"),
            (1_000_000_000).to_bytes(8, "big"),
            (21_000).to_bytes(8, "big"),
            (36_000_000).to_bytes(8, "big"),
            state_diff,
        ]
    )

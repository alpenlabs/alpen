"""Exercise crafted EE DA transport lifecycle cases."""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services import BitcoinService
from envconfigs.alpen_client import AlpenClientEnv
from tests.alpen_client.ee_da.codec import reassemble_and_validate_blobs
from tests.alpen_client.ee_da.helpers import observe_da_transport, scan_for_da_envelopes
from tests.alpen_client.ee_da.injection import (
    broadcast_raw_tx,
    mine_blocks,
    post_ee_da_envelope,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestDaTransportLifecycleTest(BaseTest):
    """Crafted DA envelopes exercise malformed, incomplete, and ordered scans."""

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

        self._malformed_blob_is_observed_but_not_reassembled(bitcoin, btc_rpc)
        self._incomplete_commit_is_observed_but_not_emitted(bitcoin, btc_rpc)
        self._commit_only_incomplete_is_observed_but_not_emitted(bitcoin, btc_rpc)
        self._orphan_reveal_window_reports_missing_commit(bitcoin, btc_rpc)
        self._out_of_order_reveals_reassemble_by_commit_output(bitcoin, btc_rpc)
        return True

    def _malformed_blob_is_observed_but_not_reassembled(self, bitcoin, btc_rpc) -> None:
        start_height = btc_rpc.proxy.getblockcount() + 1
        malformed = post_ee_da_envelope(bitcoin, chunks=[b"not a DA blob"])
        honest_blob = _make_da_blob(update_seq_no=7, block_num=42, state_diff=b"honest diff")
        honest = post_ee_da_envelope(
            bitcoin,
            chunks=[honest_blob[:24], honest_blob[24:]],
        )
        end_height = btc_rpc.proxy.getblockcount()

        observation = observe_da_transport(btc_rpc, start_height, end_height)
        malformed_commit = _single_commit(observation, malformed.commit_txid)
        assert malformed_commit.is_complete(), "malformed commit should be transport-complete"

        envelopes = scan_for_da_envelopes(btc_rpc, start_height, end_height)
        assert any(env.commit_txid == malformed.commit_txid for env in envelopes), (
            "complete malformed envelope should be visible to the transport scanner"
        )

        results = reassemble_and_validate_blobs(envelopes)
        result_txids = {result.commit_txid for result in results}
        assert malformed.commit_txid not in result_txids, (
            "malformed blob should be rejected by DA blob reassembly"
        )
        assert honest.commit_txid in result_txids, (
            "malformed blob should not block later honest blob reassembly"
        )

    def _incomplete_commit_is_observed_but_not_emitted(self, bitcoin, btc_rpc) -> None:
        start_height = btc_rpc.proxy.getblockcount() + 1
        incomplete = post_ee_da_envelope(
            bitcoin,
            chunks=[b"first incomplete chunk", b"missing reveal chunk"],
            reveal_count=1,
        )
        end_height = btc_rpc.proxy.getblockcount()

        observation = observe_da_transport(btc_rpc, start_height, end_height)
        commit = _single_commit(observation, incomplete.commit_txid)
        assert not commit.is_complete(), "commit with a missing reveal should be incomplete"
        assert commit.missing_vouts == [2], f"unexpected missing vouts: {commit.missing_vouts}"

        envelopes = scan_for_da_envelopes(btc_rpc, start_height, end_height)
        assert not envelopes, "incomplete envelope must not be emitted as complete DA"

    def _commit_only_incomplete_is_observed_but_not_emitted(self, bitcoin, btc_rpc) -> None:
        start_height = btc_rpc.proxy.getblockcount() + 1
        incomplete = post_ee_da_envelope(
            bitcoin,
            chunks=[b"missing first reveal", b"missing second reveal"],
            reveal_count=0,
        )
        end_height = btc_rpc.proxy.getblockcount()

        observation = observe_da_transport(btc_rpc, start_height, end_height)
        commit = _single_commit(observation, incomplete.commit_txid)
        assert not commit.is_complete(), "commit-only envelope should be incomplete"
        assert commit.missing_vouts == [1, 2], (
            f"commit-only envelope should miss all reveals, got {commit.missing_vouts}"
        )

        envelopes = scan_for_da_envelopes(btc_rpc, start_height, end_height)
        assert not envelopes, "commit-only envelope must not be emitted as complete DA"

    def _orphan_reveal_window_reports_missing_commit(self, bitcoin, btc_rpc) -> None:
        start_height = btc_rpc.proxy.getblockcount() + 1
        crafted = post_ee_da_envelope(bitcoin, chunks=[_make_da_blob(8, 43, b"orphan")])
        reveal_height = start_height + 1
        assert btc_rpc.proxy.getblockcount() == reveal_height, (
            "inline mining should mine one commit block and one reveal block"
        )

        observation = observe_da_transport(btc_rpc, reveal_height, reveal_height)
        assert not observation.commits, "reveal-only window should not include a commit"
        assert len(observation.orphan_reveals) == 1, (
            f"expected one orphan reveal, got {len(observation.orphan_reveals)}"
        )
        orphan = observation.orphan_reveals[0]
        assert orphan.parent_txid == crafted.commit_txid, (
            "orphan reveal should record the missing parent commit txid"
        )
        assert orphan.parent_vout == 1, "orphan reveal should record the missing commit output"

        envelopes = scan_for_da_envelopes(btc_rpc, reveal_height, reveal_height)
        assert not envelopes, "reveal-only window must not emit a complete DA envelope"

    def _out_of_order_reveals_reassemble_by_commit_output(self, bitcoin, btc_rpc) -> None:
        blob = _make_da_blob(update_seq_no=9, block_num=44, state_diff=b"ordered by vout")
        chunks = [blob[:31], blob[31:]]

        start_height = btc_rpc.proxy.getblockcount() + 1
        crafted = post_ee_da_envelope(
            bitcoin,
            chunks=chunks,
            reveal_count=0,
            mine_mode="manual",
        )
        mine_blocks(bitcoin, 1)

        reveal_one = crafted.reveal_txs[1]
        reveal_zero = crafted.reveal_txs[0]
        broadcast_raw_tx(bitcoin, reveal_one.hex)
        mine_blocks(bitcoin, 1)
        broadcast_raw_tx(bitcoin, reveal_zero.hex)
        mine_blocks(bitcoin, 1)
        end_height = btc_rpc.proxy.getblockcount()

        envelopes = scan_for_da_envelopes(btc_rpc, start_height, end_height)
        crafted_envs = [env for env in envelopes if env.commit_txid == crafted.commit_txid]
        assert {env.chunk_index for env in crafted_envs} == {0, 1}, (
            f"expected both out-of-order chunks, got {crafted_envs}"
        )
        reveal_heights = {env.chunk_index: env.reveal_height for env in crafted_envs}
        assert reveal_heights[1] < reveal_heights[0], (
            "test setup should mine chunk 1 before chunk 0"
        )

        results = reassemble_and_validate_blobs(crafted_envs)
        assert len(results) == 1, f"expected one reassembled blob, got {len(results)}"
        assert results[0].blob.last_block_num == 44
        assert results[0].blob.state_diff == b"ordered by vout"


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


def _single_commit(observation, commit_txid: str):
    matches = [commit for commit in observation.commits if commit.commit_txid == commit_txid]
    assert len(matches) == 1, f"expected one observation for {commit_txid}, got {len(matches)}"
    return matches[0]

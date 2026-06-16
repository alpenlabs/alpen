"""
Test orchestration helpers for DA pipeline testing.

Provides L1 scanning and batch sealing triggers.
"""

import logging
import time
from dataclasses import dataclass

from envconfigs.alpen_client import DEFAULT_DA_MAGIC_BYTES
from tests.alpen_client.ee_da.codec import (
    DaEnvelope,
    extract_chunk_from_reveal_witness,
    parse_commit_op_return,
)

logger = logging.getLogger(__name__)

EXPECTED_MAGIC_BYTES = DEFAULT_DA_MAGIC_BYTES


@dataclass
class RevealObservation:
    """Reveal-like tx observed in the scanned L1 window."""

    reveal_txid: str
    reveal_wtxid: str
    reveal_height: int
    parent_txid: str
    parent_vout: int
    chunk_payload: bytes


@dataclass
class CommitObservation:
    """Transport-level DA commit observation, including incomplete commits."""

    commit_txid: str
    commit_height: int
    expected_vouts: list[int]
    reveals: list[DaEnvelope]
    missing_vouts: list[int]

    def is_complete(self) -> bool:
        return len(self.expected_vouts) > 0 and not self.missing_vouts


@dataclass
class DaScanObservation:
    """Transport-level DA scan result for complete and incomplete envelopes."""

    commits: list[CommitObservation]
    orphan_reveals: list[RevealObservation]

    def complete_envelopes(self) -> list[DaEnvelope]:
        envelopes: list[DaEnvelope] = []
        for commit in self.commits:
            if not commit.is_complete():
                continue
            total = len(commit.expected_vouts)
            for env in commit.reveals:
                env.total_chunks = total
            envelopes.extend(commit.reveals)
        return envelopes


def observe_da_transport(
    btc_rpc,
    start_height: int,
    end_height: int,
    magic_bytes: bytes = EXPECTED_MAGIC_BYTES,
) -> DaScanObservation:
    """Observe DA commit/reveal transport state in an L1 window.

    Unlike `scan_for_da_envelopes`, this reports incomplete commits and
    reveal-like txs whose parent commit is outside the scanned window. It
    shares the same magic filter, txid normalization, and canonical L1 view as
    the complete-envelope scanner.
    """
    blocks_by_tx, tx_height = _scan_l1_window(btc_rpc, start_height, end_height)
    reveal_observations = _observe_reveals(blocks_by_tx, tx_height)
    reveals_by_parent = {
        (reveal.parent_txid, reveal.parent_vout): reveal for reveal in reveal_observations
    }

    commit_txids: set[str] = set()
    commits: list[CommitObservation] = []
    for txid, tx in blocks_by_tx.items():
        if _is_coinbase(tx):
            continue
        outputs = tx.get("vout") or []
        if not outputs:
            continue

        op_return_hex = outputs[0]["scriptPubKey"].get("hex", "")
        commit = parse_commit_op_return(op_return_hex, magic_bytes)
        if commit is None:
            continue

        commit_txids.add(txid)
        commit_height = tx_height[txid]
        expected_vouts: list[int] = []
        reveals: list[DaEnvelope] = []
        missing_vouts: list[int] = []

        for vout, out in enumerate(outputs[1:], start=1):
            spk_type = out["scriptPubKey"].get("type", "")
            if spk_type != "witness_v1_taproot":
                break

            expected_vouts.append(vout)
            reveal = reveals_by_parent.get((txid, vout))
            if reveal is None:
                missing_vouts.append(vout)
                continue

            reveals.append(
                DaEnvelope(
                    commit_txid=txid,
                    commit_height=commit_height,
                    total_chunks=0,
                    reveal_txid=reveal.reveal_txid,
                    reveal_wtxid=reveal.reveal_wtxid,
                    reveal_height=reveal.reveal_height,
                    reveal_spent_txid=reveal.parent_txid,
                    reveal_spent_vout=reveal.parent_vout,
                    chunk_index=vout - 1,
                    chunk_payload=reveal.chunk_payload,
                )
            )

        commits.append(
            CommitObservation(
                commit_txid=txid,
                commit_height=commit_height,
                expected_vouts=expected_vouts,
                reveals=reveals,
                missing_vouts=missing_vouts,
            )
        )

    orphan_reveals = [
        reveal for reveal in reveal_observations if reveal.parent_txid not in commit_txids
    ]

    return DaScanObservation(commits=commits, orphan_reveals=orphan_reveals)


def scan_for_da_envelopes(
    btc_rpc,
    start_height: int,
    end_height: int,
    magic_bytes: bytes = EXPECTED_MAGIC_BYTES,
) -> list[DaEnvelope]:
    """Scan L1 blocks for DA chunked-envelope reveals across `[start, end]`.

    Walks every block in the range and identifies commit txs by an
    OP_RETURN at output 0 carrying `magic ++ version(4)`.
    Each commit has dynamic-many P2TR outputs; for every P2TR output
    `vout`, the matching reveal tx is the one that spends `(commit_txid,
    vout)`. The scanner reads reveals from the witness of those spending
    txs.

    The writer waits for a multi-reveal commit to confirm before
    broadcasting any reveals, so the commit and its reveals routinely
    land in **different** L1 blocks. To pair them correctly across
    scan-window boundaries this scanner must always be called with
    `start_height` covering the full range from the first DA-bearing
    block (e.g. baseline_l1_height) up to the current tip — never an
    incremental delta. Callers should rebuild the result list each pass
    rather than appending, since the scanner re-emits every envelope
    whose commit + all reveals are now visible. Envelopes whose reveals
    have not yet confirmed are simply omitted; they will appear on a
    later scan once the reveals are mined.
    """
    return observe_da_transport(btc_rpc, start_height, end_height, magic_bytes).complete_envelopes()


def _scan_l1_window(btc_rpc, start_height: int, end_height: int):
    blocks_by_tx: dict[str, dict] = {}
    tx_height: dict[str, int] = {}

    for height in range(start_height, end_height + 1):
        block_hash = btc_rpc.proxy.getblockhash(height)
        block = btc_rpc.proxy.getblock(block_hash, 2)
        for tx in block["tx"]:
            blocks_by_tx[tx["txid"]] = tx
            tx_height[tx["txid"]] = height
    return blocks_by_tx, tx_height


def _observe_reveals(blocks_by_tx: dict[str, dict], tx_height: dict[str, int]):
    observations: list[RevealObservation] = []
    for txid, tx in blocks_by_tx.items():
        if _is_coinbase(tx) or len(tx["vin"]) != 1:
            continue

        vin = tx["vin"][0]
        txinwitness = vin.get("txinwitness") or []
        chunk_payload = extract_chunk_from_reveal_witness(txinwitness)
        if chunk_payload is None:
            continue

        observations.append(
            RevealObservation(
                reveal_txid=txid,
                reveal_wtxid=tx.get("hash", txid),
                reveal_height=tx_height[txid],
                parent_txid=vin["txid"],
                parent_vout=vin["vout"],
                chunk_payload=chunk_payload,
            )
        )
    return observations


def _is_coinbase(tx: dict) -> bool:
    return bool(tx.get("vin")) and "coinbase" in tx["vin"][0]


EE_DA_BLOCK_WAIT_SECONDS = 15.0


def trigger_batch_sealing(sequencer, btc_rpc, num_blocks: int = 8):
    """Wait for blocks and mine L1 to trigger batch sealing and DA posting.

    EE DA tests use short batch windows so they can exercise sealing and
    publishing without waiting for production-sized block ranges. The
    per-block budget reflects debug Reth payload import time in functional
    tests, not the configured sequencer target cadence.
    """
    sequencer.wait_for_additional_blocks(
        num_blocks,
        timeout_per_block=EE_DA_BLOCK_WAIT_SECONDS,
        timeout_slack=45,
    )

    mine_address = btc_rpc.proxy.getnewaddress()
    btc_rpc.proxy.generatetoaddress(10, mine_address)
    time.sleep(5)
    btc_rpc.proxy.generatetoaddress(2, mine_address)

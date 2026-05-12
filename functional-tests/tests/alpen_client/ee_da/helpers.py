"""
Test orchestration helpers for DA pipeline testing.

Provides L1 scanning and batch sealing triggers.
"""

import logging
import time

from envconfigs.alpen_client import DEFAULT_DA_MAGIC_BYTES
from tests.alpen_client.ee_da.codec import (
    DaEnvelope,
    extract_chunk_from_reveal_witness,
    parse_commit_op_return,
)

logger = logging.getLogger(__name__)

EXPECTED_MAGIC_BYTES = DEFAULT_DA_MAGIC_BYTES


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
    blocks_by_tx: dict[str, dict] = {}
    spent_by: dict[tuple[str, int], dict] = {}
    tx_height: dict[str, int] = {}

    for height in range(start_height, end_height + 1):
        block_hash = btc_rpc.proxy.getblockhash(height)
        block = btc_rpc.proxy.getblock(block_hash, 2)
        for tx in block["tx"]:
            blocks_by_tx[tx["txid"]] = tx
            tx_height[tx["txid"]] = height
            for vin in tx["vin"]:
                if "coinbase" in vin:
                    continue
                spent_by[(vin["txid"], vin["vout"])] = tx

    envelopes: list[DaEnvelope] = []

    for txid, tx in blocks_by_tx.items():
        if "coinbase" in tx["vin"][0]:
            continue
        outputs = tx.get("vout") or []
        if not outputs:
            continue

        # Output 0 must be the commit OP_RETURN.
        op_return_hex = outputs[0]["scriptPubKey"].get("hex", "")
        commit = parse_commit_op_return(op_return_hex, magic_bytes)
        if commit is None:
            continue

        # Walk subsequent P2TR outputs and look up the spending reveal tx.
        # If any reveal is missing from our scan window, the envelope is
        # not yet fully observed; skip emitting it (a later scan with a
        # wider range will pick it up).
        commit_height = tx_height[txid]
        chunks: list[DaEnvelope] = []
        complete = True
        for vout, out in enumerate(outputs[1:], start=1):
            spk_type = out["scriptPubKey"].get("type", "")
            if spk_type != "witness_v1_taproot":
                # First non-P2TR output marks the change/end. Stop here.
                break

            reveal = spent_by.get((txid, vout))
            if reveal is None:
                # The reveal hasn't confirmed yet (or is outside the scan
                # window). Drop this envelope from this pass.
                complete = False
                break

            if len(reveal["vin"]) != 1:
                logger.debug(
                    "skipping commit %s vout %d: reveal has %d inputs (expected 1)",
                    txid,
                    vout,
                    len(reveal["vin"]),
                )
                complete = False
                break
            txinwitness = reveal["vin"][0].get("txinwitness") or []
            chunk_payload = extract_chunk_from_reveal_witness(txinwitness)
            if chunk_payload is None:
                logger.debug(
                    "skipping commit %s vout %d: could not extract chunk from witness",
                    txid,
                    vout,
                )
                complete = False
                break

            chunks.append(
                DaEnvelope(
                    commit_txid=txid,
                    commit_height=commit_height,
                    total_chunks=0,  # filled in below
                    reveal_txid=reveal["txid"],
                    reveal_wtxid=reveal.get("hash", reveal["txid"]),
                    reveal_height=tx_height.get(reveal["txid"], commit_height),
                    reveal_spent_txid=reveal["vin"][0]["txid"],
                    reveal_spent_vout=reveal["vin"][0]["vout"],
                    chunk_index=vout - 1,
                    chunk_payload=chunk_payload,
                )
            )

        if not complete or not chunks:
            continue

        total = len(chunks)
        for env in chunks:
            env.total_chunks = total
        envelopes.extend(chunks)

    return envelopes


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

"""
Test orchestration helpers for DA pipeline testing.

Provides L1 scanning, batch sealing triggers, and shared test state.
"""

import logging
import time
from dataclasses import dataclass, field

from envconfigs.alpen_client import DEFAULT_DA_MAGIC_BYTES
from tests.alpen_client.ee_da.codec import (
    ZERO_WTXID,
    DaBlob,
    DaEnvelope,
    extract_envelope_payload,
    extract_prev_tail_wtxid,
    parse_da_chunk_header,
    parse_op_return_data,
)

logger = logging.getLogger(__name__)

EXPECTED_MAGIC_BYTES = DEFAULT_DA_MAGIC_BYTES


@dataclass
class TestState:
    """
    Holds state across test scenarios for validation.

    Tracks seen blobs and wtxid chain to validate:
    1. last_block_num progression (monotonically increasing)
    2. Wtxid chain integrity (each envelope references the previous)
    """

    # Max last_block_num seen so far (for scenario separation)
    max_block_num: int = 0
    # All DA envelopes seen, ordered by L1 height
    envelopes: list[DaEnvelope] = field(default_factory=list)
    # All reassembled blobs with their last_block_num
    blobs: list[DaBlob] = field(default_factory=list)
    # Baseline L1 height at test start
    baseline_l1_height: int = 0


def scan_for_da_envelopes(
    btc_rpc,
    start_height: int,
    end_height: int,
    magic_bytes: bytes = EXPECTED_MAGIC_BYTES,
) -> list[DaEnvelope]:
    """
    Scan L1 blocks for DA envelopes with matching magic bytes.

    Returns DaEnvelope objects with full metadata including prev_tail_wtxid
    for chain validation.
    """
    envelopes = []

    for height in range(start_height, end_height + 1):
        block_hash = btc_rpc.proxy.getblockhash(height)
        block = btc_rpc.proxy.getblock(block_hash, 2)

        for tx in block["tx"]:
            if "coinbase" in tx["vin"][0]:
                continue

            for vout in tx["vout"]:
                script_hex = vout["scriptPubKey"].get("hex", "")
                if not script_hex.startswith("6a"):
                    continue

                op_return_data = parse_op_return_data(script_hex)
                if not op_return_data or op_return_data[:4] != magic_bytes:
                    continue

                # Extract prev_tail_wtxid from OP_RETURN
                prev_tail_wtxid = extract_prev_tail_wtxid(op_return_data)

                # Extract payload from witness
                payload = None
                for vin in tx["vin"]:
                    if "txinwitness" in vin and len(vin["txinwitness"]) >= 2:
                        envelope_script = bytes.fromhex(vin["txinwitness"][1])
                        payload = extract_envelope_payload(envelope_script)
                        break

                if payload:
                    header = parse_da_chunk_header(payload)
                    if header:
                        # In Bitcoin Core RPC, the wtxid is in the "hash" field,
                        # not "wtxid". The "hash" field is HASH256 of full tx
                        # including witness data, in display (reversed) byte order.
                        envelopes.append(
                            DaEnvelope(
                                txid=tx["txid"],
                                wtxid=tx.get("hash", tx["txid"]),
                                height=height,
                                payload=payload,
                                blob_hash=header.blob_hash,
                                chunk_index=header.chunk_index,
                                total_chunks=header.total_chunks,
                                prev_tail_wtxid=prev_tail_wtxid or ZERO_WTXID,
                            )
                        )

    return envelopes


def trigger_batch_sealing(sequencer, btc_rpc, num_blocks: int = 35):
    """Wait for blocks and mine L1 to trigger batch sealing and DA posting.

    With batch_sealing_block_count=30, we need at least 30 blocks to seal
    a batch plus some extra to trigger the next batch (which posts DA for
    the previous batch). Default of 35 blocks ensures at least one batch
    is sealed.
    """
    initial_block = sequencer.get_block_number()
    target_block = initial_block + num_blocks

    sequencer.wait_for_block(target_block, timeout=180)

    mine_address = btc_rpc.proxy.getnewaddress()
    btc_rpc.proxy.generatetoaddress(10, mine_address)
    time.sleep(5)
    btc_rpc.proxy.generatetoaddress(2, mine_address)

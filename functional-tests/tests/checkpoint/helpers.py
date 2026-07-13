"""Checkpoint test helpers: duty polling, epoch parsing, and DA payload extraction."""

import json
import logging
from dataclasses import dataclass
from pathlib import Path

from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from tests.alpen_client.ee_da.codec import extract_envelope_payload

logger = logging.getLogger(__name__)

CHECKPOINT_SUBPROTOCOL_ID = 1
OL_STF_CHECKPOINT_TX_TYPE = 1


# ---------------------------------------------------------------------------
# Sequencer signer checkpoint duty helpers
# ---------------------------------------------------------------------------


def wait_for_checkpoint_duty(
    admin_rpc,
    status_rpc=None,
    timeout: int = 60,
    step: float = 1.0,
    min_epoch: int | None = None,
):
    """Wait until getSequencerDuties returns a SignCheckpoint duty.

    When *min_epoch* is set, duties for earlier epochs are skipped.
    When *min_epoch* is None, waits for duty at or beyond the next epoch.
    """
    if min_epoch is None:
        if status_rpc is None:
            raise AssertionError("status_rpc is required when min_epoch is not provided")
        status = status_rpc.strata_getChainStatus()
        tip = status.get("tip")
        if not isinstance(tip, dict) or not isinstance(tip.get("epoch"), int):
            raise AssertionError(f"Unable to determine current epoch from chain status: {status}")

        min_epoch = tip["epoch"] + 1

    def _get_duty():
        duties = admin_rpc.strata_strataadmin_getSequencerDuties()
        for duty in duties:
            if isinstance(duty, dict) and "SignCheckpoint" in duty:
                if parse_checkpoint_epoch(duty) < min_epoch:
                    continue
                return duty
        return None

    return wait_until_with_value(
        _get_duty,
        lambda duty: duty is not None,
        error_with="Timed out waiting for SignCheckpoint duty",
        timeout=timeout,
        step=step,
    )


def mine_until_finalized_epoch(
    bitcoin: BitcoinService,
    strata: StrataService,
    strata_rpc,
    target_epoch: int,
    timeout: int = 120,
    step: float = 1.0,
) -> dict:
    """Mine L1 blocks until finalized epoch reaches target_epoch."""

    def _check():
        return strata.get_sync_status(strata_rpc).get("finalized")

    def _is_finalized(v):
        return (
            isinstance(v, dict)
            and v.get("epoch", -1) >= target_epoch
            and v.get("last_blkid") != "00" * 32
        )

    return bitcoin.mine_until(
        check=_check,
        predicate=_is_finalized,
        error_with=f"Finalized epoch did not reach {target_epoch}",
        timeout=timeout,
        step=step,
    )


# ---------------------------------------------------------------------------
# Checkpoint payload parsing
# ---------------------------------------------------------------------------


def parse_checkpoint_epoch(duty: dict) -> int:
    """Extract epoch from the duty's SSZ-encoded CheckpointPayload."""
    return parse_checkpoint_payload(bytes(duty["SignCheckpoint"]["checkpoint"])).epoch


@dataclass
class CheckpointPayloadView:
    """Parsed view over SSZ-encoded CheckpointPayload bytes.

    `new_tip_bytes` is the raw 48-byte fixed CheckpointTip region, usable for
    direct equality anchoring; `ol_state_diff` is the inner strata-codec
    StateDiff (OLDaPayloadV1) bytes from the sidecar.
    """

    epoch: int
    l1_height: int
    l2_slot: int
    l2_blkid_hex: str
    new_tip_bytes: bytes
    ol_state_diff: bytes


# SSZ layout of CheckpointPayload (see asm checkpoint/types/ssz/payload.ssz):
#   [0..48)   new_tip: CheckpointTip (epoch u32 LE, l1_height u32 LE, l2 slot u64 LE, blkid 32B)
#   [48..52)  offset of sidecar (variable)
#   [52..56)  offset of proof (variable)
# CheckpointSidecar (offsets relative to sidecar start):
#   [0..4)    offset of ol_state_diff
#   [4..8)    offset of ol_logs
#   [8..112)  terminal_header_complement (fixed 104B)
_TIP_LEN = 48
_PAYLOAD_FIXED_LEN = 56


def _read_u32(data: bytes, at: int) -> int:
    return int.from_bytes(data[at : at + 4], "little")


def parse_checkpoint_payload(payload: bytes) -> CheckpointPayloadView:
    """Parse SSZ-encoded CheckpointPayload bytes into a CheckpointPayloadView."""
    assert len(payload) >= _PAYLOAD_FIXED_LEN, f"payload too short: {len(payload)}"

    sidecar_off = _read_u32(payload, 48)
    proof_off = _read_u32(payload, 52)
    assert _PAYLOAD_FIXED_LEN <= sidecar_off <= proof_off <= len(payload), (
        f"bad payload offsets: sidecar={sidecar_off} proof={proof_off} len={len(payload)}"
    )

    sidecar = payload[sidecar_off:proof_off]
    diff_off = _read_u32(sidecar, 0)
    logs_off = _read_u32(sidecar, 4)
    assert 8 <= diff_off <= logs_off <= len(sidecar), (
        f"bad sidecar offsets: diff={diff_off} logs={logs_off} len={len(sidecar)}"
    )

    return CheckpointPayloadView(
        epoch=_read_u32(payload, 0),
        l1_height=_read_u32(payload, 4),
        l2_slot=int.from_bytes(payload[8:16], "little"),
        l2_blkid_hex=payload[16:48].hex(),
        new_tip_bytes=payload[:_TIP_LEN],
        ol_state_diff=sidecar[diff_off:logs_off],
    )


def verify_payload_parser_fixture():
    """Self-check parse_checkpoint_payload against the Rust-generated SSZ fixture
    (see `dump_checkpoint_payload_fixture` in crates/test-utils/l2)."""
    fixture_path = Path(__file__).parent / "checkpoint_payload_fixture.json"
    fixture = json.loads(fixture_path.read_text())
    view = parse_checkpoint_payload(bytes.fromhex(fixture["payload_ssz_hex"]))
    assert view.epoch == fixture["epoch"]
    assert view.l1_height == fixture["l1_height"]
    assert view.l2_slot == fixture["l2_slot"]
    assert view.l2_blkid_hex == fixture["l2_blkid_hex"]
    assert view.ol_state_diff.hex() == fixture["ol_state_diff_hex"]


def _decode_codec_varint(data: bytes, at: int) -> tuple[int, int]:
    """Decode a strata-codec varint; returns (value, next_index)."""
    first = data[at]
    match first >> 6:
        case 0 | 1:
            return first, at + 1
        case 2:
            return int.from_bytes(bytes([first & 0x3F, data[at + 1]]), "big"), at + 2
        case _:
            value = int.from_bytes(bytes([first & 0x3F]) + data[at + 1 : at + 4], "big")
            return value, at + 4


def extract_posted_checkpoint_payload(btc_rpc, txid: str) -> bytes:
    """Extract SSZ CheckpointPayload bytes from a posted L1 checkpoint tx.

    Verifies the SPS-50 OP_RETURN tag (subprotocol=1, tx_type=1) on output 0,
    then pulls the envelope payload (varint-length-prefixed SSZ) out of the
    input-0 tapscript witness.
    """
    tx = btc_rpc.proxy.getrawtransaction(txid, 1)

    spk_hex = tx["vout"][0]["scriptPubKey"]["hex"]
    spk = bytes.fromhex(spk_hex)
    assert spk[:1] == b"\x6a", f"output 0 is not OP_RETURN: {spk_hex}"
    # Skip OP_RETURN + push opcode (single-byte push for tag lengths <= 75).
    tag = spk[2:]
    assert len(tag) >= 6, f"SPS-50 tag too short: {spk_hex}"
    assert tag[4] == CHECKPOINT_SUBPROTOCOL_ID, f"unexpected subprotocol id: {tag[4]}"
    assert tag[5] == OL_STF_CHECKPOINT_TX_TYPE, f"unexpected tx type: {tag[5]}"

    witness = tx["vin"][0].get("txinwitness")
    assert witness and len(witness) >= 2, f"checkpoint tx {txid} has no tapscript witness"
    # Script is second-to-last witness element; control block is last.
    envelope = extract_envelope_payload(bytes.fromhex(witness[-2]))
    assert envelope is not None, f"no envelope payload in checkpoint tx {txid}"

    ssz_len, start = _decode_codec_varint(envelope, 0)
    assert start + ssz_len == len(envelope), (
        f"envelope length prefix mismatch: prefix={ssz_len}, actual={len(envelope) - start}"
    )
    return envelope[start:]

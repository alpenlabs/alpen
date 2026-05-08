"""DA blob codec: data structures, parsing, reassembly, and validation.

Wire format:

  Commit tx:
    output 0:    OP_RETURN <magic(4) ++ version(4)>
    outputs 1..: P2TR funding one reveal each (chunk_index = vout - 1)
    last out:    optional change

  Reveal tx (one per chunk):
    input 0:     spends commit.output[i+1] via tapscript path
    witness:     [schnorr_sig, tapscript, control_block]
                 tapscript = <sequencer_pk> OP_CHECKSIG
                             OP_FALSE OP_IF <chunk_bytes> OP_ENDIF
    output 0:    dust to sequencer

Reveals carry no OP_RETURN; chunk_index/total are implicit in commit-output
ordering. Reveals are independent across batches — no wtxid chain.
"""

import logging
from dataclasses import dataclass

logger = logging.getLogger(__name__)

# Commit OP_RETURN payload: magic(4) + version(4) = 8 bytes.
COMMIT_OP_RETURN_PAYLOAD_LEN = 8
COMMIT_OP_RETURN_VERSION = 0

# Minimum state_diff size for empty batch (3 u32 counts, 4 bytes BE each).
EMPTY_STATE_DIFF_MAX_SIZE = 12


# =============================================================================
# DATA STRUCTURES
# =============================================================================


@dataclass
class CommitOpReturn:
    """Parsed commit-tx OP_RETURN payload."""

    magic: bytes
    version: int


@dataclass
class EvmHeaderDigest:
    """Parsed EVM header digest from the DA blob."""

    block_num: int
    timestamp: int
    base_fee: int
    gas_used: int
    gas_limit: int


@dataclass
class DaBlob:
    """Parsed DA blob structure from strata-codec encoding."""

    update_seq_no: int
    evm_header: EvmHeaderDigest
    state_diff: bytes

    @property
    def last_block_num(self) -> int:
        return self.evm_header.block_num

    def is_empty_batch(self) -> bool:
        """Returns True if this batch has no state changes."""
        return len(self.state_diff) <= EMPTY_STATE_DIFF_MAX_SIZE


@dataclass
class DaEnvelope:
    """One reveal carrying a single chunk of a DA blob.

    Version, magic, and total_chunks are derived from the commit tx that funded
    this reveal — so several DaEnvelope rows from the same commit share those
    fields. `chunk_index` is `vout - 1` of the commit output this reveal spends.
    """

    # Commit-tx-level metadata (same for every chunk of a blob).
    commit_txid: str
    commit_height: int
    total_chunks: int

    # Per-reveal data.
    reveal_txid: str
    reveal_wtxid: str
    reveal_height: int
    reveal_spent_txid: str
    reveal_spent_vout: int
    chunk_index: int
    chunk_payload: bytes


@dataclass
class ReassembledBlob:
    """Result of blob reassembly with validation metadata."""

    blob: DaBlob
    total_chunks: int
    chunk_sizes: list[int]
    total_size: int
    commit_txid: str


# =============================================================================
# PARSING
# =============================================================================


def parse_commit_op_return(script_hex: str, expected_magic: bytes) -> CommitOpReturn | None:
    """Parse a commit-tx OP_RETURN script.

    Layout: OP_RETURN <push8: magic(4) ++ version(4)>.
    Returns None if the script is not OP_RETURN, the push length is wrong,
    or the magic doesn't match.
    """
    script = bytes.fromhex(script_hex)
    if len(script) != 2 + COMMIT_OP_RETURN_PAYLOAD_LEN:
        return None
    if script[0] != 0x6A:
        return None
    if script[1] != COMMIT_OP_RETURN_PAYLOAD_LEN:
        return None

    payload = script[2:]
    magic = payload[:4]
    if magic != expected_magic:
        return None
    version = int.from_bytes(payload[4:8], "big")
    if version != COMMIT_OP_RETURN_VERSION:
        return None

    return CommitOpReturn(
        magic=magic,
        version=version,
    )


def parse_evm_header_digest(data: bytes) -> EvmHeaderDigest | None:
    """Parse EvmHeaderDigest (40 bytes, 5 x u64 big-endian)."""
    if len(data) < 40:
        return None
    return EvmHeaderDigest(
        block_num=int.from_bytes(data[0:8], "big"),
        timestamp=int.from_bytes(data[8:16], "big"),
        base_fee=int.from_bytes(data[16:24], "big"),
        gas_used=int.from_bytes(data[24:32], "big"),
        gas_limit=int.from_bytes(data[32:40], "big"),
    )


def parse_da_blob(data: bytes) -> DaBlob | None:
    """Parse DaBlob from strata-codec encoded bytes."""
    if len(data) < 48:
        return None
    evm_header = parse_evm_header_digest(data[8:48])
    if evm_header is None:
        return None
    return DaBlob(
        update_seq_no=int.from_bytes(data[0:8], "big"),
        evm_header=evm_header,
        state_diff=data[48:],
    )


def extract_envelope_payload(script: bytes) -> bytes | None:
    """Extract chunk payload from the reveal tapscript.

    Tapscript shape:
        <pubkey(32)> OP_CHECKSIG OP_FALSE OP_IF <chunk_bytes> OP_ENDIF

    The pubkey + OP_CHECKSIG prefix is ignored; we read every push between
    OP_FALSE OP_IF and OP_ENDIF and concatenate them.
    """
    OP_FALSE, OP_IF, OP_ENDIF = 0x00, 0x63, 0x68
    OP_PUSHDATA1, OP_PUSHDATA2 = 0x4C, 0x4D

    i = 0
    while i < len(script) - 1:
        if script[i] == OP_FALSE and script[i + 1] == OP_IF:
            i += 2
            break
        i += 1
    else:
        return None

    chunks: list[bytes] = []
    while i < len(script) and script[i] != OP_ENDIF:
        opcode = script[i]
        if 0x01 <= opcode <= 0x4B:
            i += 1
            if i + opcode > len(script):
                return None
            chunks.append(script[i : i + opcode])
            i += opcode
        elif opcode == OP_PUSHDATA1:
            i += 1
            if i >= len(script):
                return None
            length = script[i]
            i += 1
            if i + length > len(script):
                return None
            chunks.append(script[i : i + length])
            i += length
        elif opcode == OP_PUSHDATA2:
            i += 1
            if i + 2 > len(script):
                return None
            length = int.from_bytes(script[i : i + 2], "little")
            i += 2
            if i + length > len(script):
                return None
            chunks.append(script[i : i + length])
            i += length
        else:
            i += 1

    return b"".join(chunks) if chunks else None


def extract_chunk_from_reveal_witness(txinwitness: list[str]) -> bytes | None:
    """Pull the chunk payload out of a reveal's tapscript witness stack.

    Witness stack for a tapscript script-path spend is
    `[<sig...>, <script>, <control_block>]`. The script (second-to-last
    element) is the tapscript carrying the chunk in OP_FALSE OP_IF...OP_ENDIF.
    """
    if len(txinwitness) < 2:
        return None
    # Script is second-to-last; control block is last.
    script_hex = txinwitness[-2]
    return extract_envelope_payload(bytes.fromhex(script_hex))


# =============================================================================
# REASSEMBLY
# =============================================================================


def reassemble_blobs_from_envelopes(envelopes: list[DaEnvelope]) -> list[DaBlob]:
    """Reassemble DaBlobs from DA envelopes (caller-friendly wrapper)."""
    return [r.blob for r in reassemble_and_validate_blobs(envelopes)]


def reassemble_and_validate_blobs(envelopes: list[DaEnvelope]) -> list[ReassembledBlob]:
    """Reassemble DaBlobs from DA envelopes with full validation.

    Validates:
    - All chunks of a blob agree on `total_chunks`.
    - Chunk indices are sequential 0..total_chunks-1.
    """
    envs_by_blob: dict[str, list[DaEnvelope]] = {}
    for env in envelopes:
        envs_by_blob.setdefault(env.commit_txid, []).append(env)

    results: list[ReassembledBlob] = []
    for commit_txid, blob_envs in envs_by_blob.items():
        blob_envs.sort(key=lambda e: e.chunk_index)

        totals = {e.total_chunks for e in blob_envs}
        if len(totals) != 1:
            logger.warning(
                "inconsistent total_chunks for commit %s: %s",
                commit_txid,
                totals,
            )
            continue
        total_chunks = next(iter(totals))

        chunk_indices = [e.chunk_index for e in blob_envs]
        if chunk_indices != list(range(total_chunks)):
            logger.warning(
                "missing or duplicate chunks for commit %s: expected %s, got %s",
                commit_txid,
                list(range(total_chunks)),
                chunk_indices,
            )
            continue

        chunk_payloads = [e.chunk_payload for e in blob_envs]
        chunk_sizes = [len(p) for p in chunk_payloads]
        full_blob = b"".join(chunk_payloads)
        total_size = len(full_blob)

        da_blob = parse_da_blob(full_blob)
        if not da_blob:
            logger.warning("failed to parse blob for commit %s", commit_txid)
            continue

        results.append(
            ReassembledBlob(
                blob=da_blob,
                total_chunks=total_chunks,
                chunk_sizes=chunk_sizes,
                total_size=total_size,
                commit_txid=commit_txid,
            )
        )

    return results


# =============================================================================
# VALIDATION
# =============================================================================


def validate_multi_chunk_blob(
    result: ReassembledBlob,
    min_chunks: int = 5,
    max_chunk_size: int = 395_000,
) -> tuple[bool, list[str]]:
    """Validate a multi-chunk blob meets expected criteria."""
    messages: list[str] = []
    is_valid = True

    if result.total_chunks < min_chunks:
        messages.append(f"FAIL: Expected at least {min_chunks} chunks, got {result.total_chunks}")
        is_valid = False
    else:
        messages.append(f"OK: Chunk count {result.total_chunks} >= {min_chunks}")

    for i, size in enumerate(result.chunk_sizes):
        if size > max_chunk_size:
            messages.append(f"FAIL: Chunk {i} size {size} exceeds max {max_chunk_size}")
            is_valid = False

    if result.total_chunks > 1:
        for i, size in enumerate(result.chunk_sizes[:-1]):
            if size < max_chunk_size * 0.9:
                messages.append(
                    f"WARN: Chunk {i} size {size} is less than 90% of max ({max_chunk_size})"
                )

    messages.append(f"INFO: Commit tx: {result.commit_txid}")
    messages.append(f"INFO: Total blob size: {result.total_size} bytes")
    messages.append(f"INFO: Chunk sizes: {result.chunk_sizes}")

    return is_valid, messages


def validate_commit_independence(envelopes: list[DaEnvelope]) -> tuple[bool, list[str]]:
    """Verify reveals are independent under the new format.

    Concretely: every reveal's input spends the *commit* tx, not a previous
    reveal. This is the property that lets fee-bumping and parallel
    publishing work without cascading.
    """
    messages: list[str] = []
    if not envelopes:
        return True, ["SKIP: no envelopes to check"]

    is_valid = True
    for env in envelopes:
        expected_vout = env.chunk_index + 1
        if env.reveal_spent_txid != env.commit_txid or env.reveal_spent_vout != expected_vout:
            messages.append(
                "FAIL: reveal "
                f"{env.reveal_txid} spends "
                f"{env.reveal_spent_txid}:{env.reveal_spent_vout}, expected "
                f"{env.commit_txid}:{expected_vout}"
            )
            is_valid = False

    if is_valid:
        reveal_txids = {e.reveal_txid for e in envelopes}
        commit_txids = {e.commit_txid for e in envelopes}
        messages.append(
            f"OK: {len(reveal_txids)} reveals across {len(commit_txids)} commit(s) "
            "all spend their own commit outputs"
        )

    return is_valid, messages

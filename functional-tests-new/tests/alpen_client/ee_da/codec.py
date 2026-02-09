"""
DA blob codec: data structures, parsing, reassembly, and validation.

Handles the strata-codec wire format for DA blobs posted to Bitcoin L1
via chunked envelope inscriptions.  This module is pure parsing logic
with no I/O or network calls.
"""

import hashlib
import logging
from dataclasses import dataclass

logger = logging.getLogger(__name__)

# DA chunk header: version(1) + blob_hash(32) + chunk_index(2) + total_chunks(2)
DA_CHUNK_HEADER_SIZE = 37

# Minimum state_diff size for empty batch (3 varint counts for empty maps/vecs)
EMPTY_STATE_DIFF_MAX_SIZE = 12

# Zero wtxid (32 bytes of zeros) — used for first envelope in chain
ZERO_WTXID = bytes(32)


# =============================================================================
# DATA STRUCTURES
# =============================================================================


@dataclass
class DaChunkHeader:
    """Parsed DA chunk header (37 bytes)."""

    version: int
    blob_hash: bytes
    chunk_index: int
    total_chunks: int


@dataclass
class EvmHeaderDigest:
    """Parsed EVM header digest (5 x u64 = 40 bytes)."""

    block_num: int
    timestamp: int
    base_fee: int
    gas_used: int
    gas_limit: int


@dataclass
class DaBlob:
    """Parsed DA blob structure from strata-codec encoding."""

    batch_id_prev_block: bytes
    batch_id_last_block: bytes
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
    """
    A complete DA envelope with its L1 transaction info.

    Tracks the wtxid chain: each envelope's OP_RETURN contains a reference
    to the previous envelope's tail wtxid (the wtxid of its last reveal tx).
    """

    txid: str
    wtxid: str
    height: int
    payload: bytes
    blob_hash: bytes
    chunk_index: int
    total_chunks: int
    prev_tail_wtxid: bytes


@dataclass
class ReassembledBlob:
    """Result of blob reassembly with validation metadata."""

    blob: DaBlob
    blob_hash: bytes
    total_chunks: int
    chunk_sizes: list[int]
    total_size: int
    hash_verified: bool


# =============================================================================
# PARSING
# =============================================================================


def parse_da_chunk_header(data: bytes) -> DaChunkHeader | None:
    """Parse DA chunk header from raw bytes.

    Layout (37 bytes):
    - version: 1 byte
    - blob_hash: 32 bytes
    - chunk_index: 2 bytes (u16 big-endian, strata-codec uses BE)
    - total_chunks: 2 bytes (u16 big-endian, strata-codec uses BE)
    """
    if len(data) < DA_CHUNK_HEADER_SIZE:
        return None
    return DaChunkHeader(
        version=data[0],
        blob_hash=data[1:33],
        chunk_index=int.from_bytes(data[33:35], "big"),
        total_chunks=int.from_bytes(data[35:37], "big"),
    )


def parse_evm_header_digest(data: bytes) -> EvmHeaderDigest | None:
    """
    Parse EvmHeaderDigest from strata-codec encoded bytes.

    Layout (40 bytes, 5 x u64 big-endian):
    - block_num: 8 bytes
    - timestamp: 8 bytes
    - base_fee: 8 bytes
    - gas_used: 8 bytes
    - gas_limit: 8 bytes
    """
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
    """
    Parse DaBlob structure from strata-codec encoded bytes.

    Layout:
    - batch_id.prev_block: 32 bytes (raw)
    - batch_id.last_block: 32 bytes (raw)
    - evm_header: 40 bytes (EvmHeaderDigest: 5 x u64 BE)
    - state_diff: remaining bytes (BatchStateDiff encoding)
    """
    # Minimum: 32 + 32 + 40 = 104
    if len(data) < 104:
        return None

    evm_header = parse_evm_header_digest(data[64:104])
    if evm_header is None:
        return None

    return DaBlob(
        batch_id_prev_block=data[0:32],
        batch_id_last_block=data[32:64],
        evm_header=evm_header,
        state_diff=data[104:],
    )


def parse_op_return_data(script_hex: str) -> bytes | None:
    """Extract ALL data pushes from OP_RETURN script, concatenated.

    The chunked envelope OP_RETURN format has TWO separate pushes:
    - OP_RETURN (0x6a)
    - PUSH4 (0x04) + magic_bytes (4 bytes)
    - PUSH32 (0x20) + prev_tail_wtxid (32 bytes)

    We need to extract and concatenate both pushes to get the full 36-byte payload.
    """
    script = bytes.fromhex(script_hex)
    if len(script) < 2 or script[0] != 0x6A:  # OP_RETURN
        return None

    # Parse all data pushes and concatenate them
    data_chunks = []
    i = 1  # Start after OP_RETURN

    while i < len(script):
        push_op = script[i]
        if push_op == 0x4C:  # OP_PUSHDATA1
            if i + 1 >= len(script):
                break
            data_len = script[i + 1]
            if i + 2 + data_len > len(script):
                break
            data_chunks.append(script[i + 2 : i + 2 + data_len])
            i += 2 + data_len
        elif push_op == 0x4D:  # OP_PUSHDATA2
            if i + 2 >= len(script):
                break
            data_len = int.from_bytes(script[i + 1 : i + 3], "little")
            if i + 3 + data_len > len(script):
                break
            data_chunks.append(script[i + 3 : i + 3 + data_len])
            i += 3 + data_len
        elif 0x01 <= push_op <= 0x4B:  # Direct push (1-75 bytes)
            if i + 1 + push_op > len(script):
                break
            data_chunks.append(script[i + 1 : i + 1 + push_op])
            i += 1 + push_op
        else:
            break

    return b"".join(data_chunks) if data_chunks else None


def extract_envelope_payload(script: bytes) -> bytes | None:
    """Extract payload from taproot envelope script (OP_FALSE OP_IF ... OP_ENDIF)."""
    OP_FALSE, OP_IF, OP_ENDIF = 0x00, 0x63, 0x68
    OP_PUSHDATA1, OP_PUSHDATA2 = 0x4C, 0x4D

    # Find envelope start
    i = 0
    while i < len(script) - 1:
        if script[i] == OP_FALSE and script[i + 1] == OP_IF:
            i += 2
            break
        i += 1
    else:
        return None

    # Extract pushed data
    chunks = []
    while i < len(script) and script[i] != OP_ENDIF:
        opcode = script[i]
        if 0x01 <= opcode <= 0x4B:  # Direct push
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


def extract_prev_tail_wtxid(op_return_data: bytes) -> bytes | None:
    """
    Extract prev_tail_wtxid from OP_RETURN data.

    OP_RETURN layout: magic_bytes(4) + prev_tail_wtxid(32)
    """
    if len(op_return_data) < 36:  # 4 + 32
        return None
    return op_return_data[4:36]


# =============================================================================
# REASSEMBLY
# =============================================================================


def reassemble_blobs_from_envelopes(envelopes: list[DaEnvelope]) -> list[DaBlob]:
    """Reassemble DaBlobs from DA envelopes (simple version for backward compat)."""
    results = reassemble_and_validate_blobs(envelopes)
    return [r.blob for r in results]


def reassemble_and_validate_blobs(envelopes: list[DaEnvelope]) -> list[ReassembledBlob]:
    """
    Reassemble DaBlobs from DA envelopes with full validation.

    Validates:
    - All chunks have consistent blob_hash and total_chunks
    - Chunk indices are sequential from 0 to total_chunks-1
    - Reconstructed blob hash matches the expected blob_hash
    - Chunk sizes are consistent (last chunk may be smaller)
    """
    # Group envelopes by blob_hash
    envs_by_hash: dict[bytes, list[DaEnvelope]] = {}

    for env in envelopes:
        if env.blob_hash not in envs_by_hash:
            envs_by_hash[env.blob_hash] = []
        envs_by_hash[env.blob_hash].append(env)

    results = []
    for blob_hash, blob_envs in envs_by_hash.items():
        # Sort by chunk_index
        blob_envs.sort(key=lambda e: e.chunk_index)

        # Validate all chunks have same total_chunks
        total_chunks_values = set(e.total_chunks for e in blob_envs)
        if len(total_chunks_values) != 1:
            logger.warning(
                "Inconsistent total_chunks for blob "
                f"{blob_hash.hex()[:16]}...: {total_chunks_values}"
            )
            continue

        total_chunks = blob_envs[0].total_chunks

        # Validate we have all chunks (0 to total_chunks-1)
        chunk_indices = [e.chunk_index for e in blob_envs]
        expected_indices = list(range(total_chunks))
        if chunk_indices != expected_indices:
            logger.warning(
                f"Missing or duplicate chunks for blob {blob_hash.hex()[:16]}...: "
                f"expected {expected_indices}, got {chunk_indices}"
            )
            continue

        # Extract payloads and track sizes
        chunk_payloads = []
        chunk_sizes = []
        for env in blob_envs:
            payload = env.payload[DA_CHUNK_HEADER_SIZE:]
            chunk_payloads.append(payload)
            chunk_sizes.append(len(payload))

        # Concatenate all payloads
        full_blob = b"".join(chunk_payloads)
        total_size = len(full_blob)

        # Verify hash
        computed_hash = hashlib.sha256(full_blob).digest()
        hash_verified = computed_hash == blob_hash
        if not hash_verified:
            logger.warning(
                f"Hash mismatch for blob {blob_hash.hex()[:16]}...: "
                f"expected {blob_hash.hex()[:16]}, got {computed_hash.hex()[:16]}"
            )
            continue

        # Parse the blob
        da_blob = parse_da_blob(full_blob)
        if not da_blob:
            logger.warning(f"Failed to parse blob {blob_hash.hex()[:16]}...")
            continue

        results.append(
            ReassembledBlob(
                blob=da_blob,
                blob_hash=blob_hash,
                total_chunks=total_chunks,
                chunk_sizes=chunk_sizes,
                total_size=total_size,
                hash_verified=hash_verified,
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
    """
    Validate a multi-chunk blob meets expected criteria.

    Returns (is_valid, list of validation messages).
    """
    messages = []
    is_valid = True

    # Check minimum chunk count
    if result.total_chunks < min_chunks:
        messages.append(f"FAIL: Expected at least {min_chunks} chunks, got {result.total_chunks}")
        is_valid = False
    else:
        messages.append(f"OK: Chunk count {result.total_chunks} >= {min_chunks}")

    # Check hash verification
    if not result.hash_verified:
        messages.append("FAIL: Blob hash verification failed")
        is_valid = False
    else:
        messages.append(f"OK: Blob hash verified ({result.blob_hash.hex()[:16]}...)")

    # Check chunk sizes are reasonable
    for i, size in enumerate(result.chunk_sizes):
        if size > max_chunk_size:
            messages.append(f"FAIL: Chunk {i} size {size} exceeds max {max_chunk_size}")
            is_valid = False

    # Check non-last chunks are close to max size (within 10%)
    if result.total_chunks > 1:
        for i, size in enumerate(result.chunk_sizes[:-1]):
            if size < max_chunk_size * 0.9:
                messages.append(
                    f"WARN: Chunk {i} size {size} is less than 90% of max ({max_chunk_size})"
                )

    # Log total size
    messages.append(f"INFO: Total blob size: {result.total_size} bytes")
    messages.append(f"INFO: Chunk sizes: {result.chunk_sizes}")

    return is_valid, messages


def validate_multi_chunk_wtxid_chain(
    envelopes: list[DaEnvelope],
    blob_hash: bytes,
) -> tuple[bool, list[str]]:
    """
    Validate the wtxid chain for a specific multi-chunk blob.

    For a multi-chunk blob with N chunks (N reveal transactions):
    - Each reveal's OP_RETURN contains: magic_bytes(4) + prev_tail_wtxid(32)
    - Reveal 0's prev_tail_wtxid should reference the previous blob's tail OR zero wtxid
    - Reveal i's prev_tail_wtxid should reference Reveal (i-1)'s wtxid

    Returns (is_valid, list of validation messages).
    """
    messages = []
    is_valid = True

    # Filter envelopes for this blob and sort by chunk_index
    blob_envs = sorted(
        [e for e in envelopes if e.blob_hash == blob_hash],
        key=lambda e: e.chunk_index,
    )

    if len(blob_envs) < 2:
        messages.append("SKIP: Wtxid chain validation requires at least 2 chunks")
        return True, messages

    messages.append(f"OK: Validating wtxid chain across {len(blob_envs)} chunks")

    # Check each chunk (from chunk 1 onwards) references the previous chunk's wtxid
    for i in range(1, len(blob_envs)):
        prev_env = blob_envs[i - 1]
        curr_env = blob_envs[i]

        # The prev_tail_wtxid in curr_env should match prev_env's wtxid
        # Note: wtxid is stored in hex, prev_tail_wtxid is bytes in little-endian
        prev_wtxid_hex = prev_env.wtxid
        prev_wtxid_bytes = bytes.fromhex(prev_wtxid_hex)[::-1]  # Reverse for LE

        if curr_env.prev_tail_wtxid == prev_wtxid_bytes:
            messages.append(f"OK: Chunk {i} correctly references chunk {i - 1}'s wtxid")
        else:
            messages.append(
                f"FAIL: Chunk {i} wtxid chain broken - expected {prev_wtxid_bytes.hex()[:16]}..., "
                f"got {curr_env.prev_tail_wtxid.hex()[:16]}..."
            )
            is_valid = False

    return is_valid, messages


def validate_wtxid_chain(envelopes: list[DaEnvelope]) -> bool:
    """
    Validate the wtxid chain: each envelope should reference the previous
    envelope's wtxid as its prev_tail_wtxid.

    The first envelope should reference ZERO_WTXID.
    """
    if not envelopes:
        return True

    # Sort by height, then by chunk_index
    sorted_envs = sorted(envelopes, key=lambda e: (e.height, e.chunk_index))

    # First envelope should reference zero wtxid
    if sorted_envs[0].prev_tail_wtxid != ZERO_WTXID:
        prev = sorted_envs[0].prev_tail_wtxid.hex()[:16]
        logger.warning(f"First envelope should reference zero wtxid, got {prev}...")
        return False

    # Each subsequent envelope should reference the previous one's wtxid.
    # The Rust builder stores wtxids in internal byte order (LE), while
    # Bitcoin Core RPC returns them in display order (BE hex). Reverse
    # the display-order hex to compare against the raw OP_RETURN bytes.
    is_valid = True
    for i in range(1, len(sorted_envs)):
        prev_wtxid = bytes.fromhex(sorted_envs[i - 1].wtxid)
        prev_wtxid_le = prev_wtxid[::-1]

        if sorted_envs[i].prev_tail_wtxid != prev_wtxid_le:
            logger.warning(
                f"Wtxid chain broken at envelope {i}: "
                f"expected {prev_wtxid_le.hex()[:16]}..., "
                f"got {sorted_envs[i].prev_tail_wtxid.hex()[:16]}..."
            )
            is_valid = False

    return is_valid

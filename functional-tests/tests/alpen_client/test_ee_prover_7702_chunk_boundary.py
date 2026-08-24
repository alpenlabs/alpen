"""Regression: EIP-7702 delegation replaced across a chunk boundary.

Repro for the chunk-prover panic "Bytecode must be present in witness":

  block N   (chunk k):    an EIP-7702 tx *sets* a delegation on authority X
  block N+1 (chunk k+1):  an EIP-7702 tx *clears* it

On the host, block N+1's payload build loads X's old delegation designator
warm from the payload builder's pre-warmed ``CachedReads`` (code attached to
the account info carried over from block N's commit), so the load never
passes through ``code_by_hash`` and reth's witness record misses it.
Post-block, X carries only the new (empty) code, so the attached-code sweep
in ``collect_accessed_codes`` misses it too. With a chunk boundary between
N and N+1, the old designator is in no block record of chunk k+1 — and the
guest's cold replay of N+1 still reads it during 7702 authorization
validation, panicking in ``WitnessDB::code_by_hash_ref``.

``chunk_sealing_block_count=1`` makes every block its own chunk, so *any*
(set, clear) pair landing in adjacent blocks crosses a chunk boundary. The
test collects a few such adjacent pairs (retrying placement as needed), then
waits for the chunk whose last block is each clearing block to reach
proof-ready in native dev-prover mode. On a node without the pre-state
witness capture fix, the chunk prover panics instead — the test fails fast
on the panic signature in the service log.
"""

import logging
import re
import time
from pathlib import Path

import flexitest
from eth_account import Account

from common.base_test import BaseTest
from common.config.constants import DEV_CHAIN_ID, DEV_PRIVATE_KEY, ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.wait import wait_until
from envconfigs.el_ol import EeOLEnv

logger = logging.getLogger(__name__)

# The 7702 authority whose delegation is set and cleared. Never sends its own
# transactions (the dev account sponsors both legs), so it needs no funds and
# its nonce advances only through applied authorizations.
AUTHORITY_PRIVATE_KEY = "0x" + "ab" * 32
AUTHORITY_ADDRESS = Account.from_key(AUTHORITY_PRIVATE_KEY).address

# Set-leg delegation target. Any address works: the designator stored on the
# authority is `0xef0100 || target` whether or not code exists there.
DELEGATE_TARGET = "0x00000000000000000000000000000000000000fe"
# Per EIP-7702, delegating to the zero address clears the designator.
ZERO_ADDRESS = "0x" + "00" * 20

# The clear tx must land in the block immediately after the set tx's block:
# the pre-warmed CachedReads only covers the parent block's state changes.
# Placement is timing-dependent (1s block time), so retry the pair.
MAX_PAIR_ATTEMPTS = 10
ADJACENT_PAIRS_TARGET = 3

# Guest panic in WitnessDB::code_by_hash_ref — surfaced through the paas
# "prove task panicked" JoinError log line on an unfixed node.
PANIC_SIGNATURE = "Bytecode must be present in witness"

CHUNK_PROOF_TIMEOUT_SECS = 240
RECEIPT_TIMEOUT_SECS = 30

# Service logs include tracing ANSI colour codes even when written to file.
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def _sign_delegation_tx(*, dev_nonce: int, auth_nonce: int, target: str, gas_price: int) -> str:
    """Sign a sponsored type-4 tx carrying one authorization for the authority."""
    authorization = Account.sign_authorization(
        {"chainId": DEV_CHAIN_ID, "address": target, "nonce": auth_nonce},
        AUTHORITY_PRIVATE_KEY,
    )
    tx = {
        "type": 4,
        "chainId": DEV_CHAIN_ID,
        "nonce": dev_nonce,
        # Intrinsic 21k + PER_EMPTY_ACCOUNT_COST 25k per authorization, with headroom.
        "gas": 120_000,
        "maxFeePerGas": gas_price * 4,
        "maxPriorityFeePerGas": min(10**9, gas_price * 4),
        "to": DEV_ACCOUNT_ADDRESS,
        "value": 0,
        "data": b"",
        "authorizationList": [authorization],
    }
    return Account.sign_transaction(tx, DEV_PRIVATE_KEY).raw_transaction.hex()


def _wait_receipt_fast(rpc, tx_hash: str) -> dict:
    """Receipt poll with a tight step, so the clear tx can chase the set tx
    into the very next block (1s block time)."""
    deadline = time.monotonic() + RECEIPT_TIMEOUT_SECS
    while time.monotonic() < deadline:
        receipt = rpc.eth_getTransactionReceipt(tx_hash)
        if receipt is not None:
            return receipt
        time.sleep(0.05)
    raise AssertionError(f"no receipt for {tx_hash} within {RECEIPT_TIMEOUT_SECS}s")


def _read_log(log_path: Path) -> str:
    if not log_path.exists():
        return ""
    return _ANSI_RE.sub("", log_path.read_bytes().decode(errors="replace"))


@flexitest.register
class TestEeProver7702ChunkBoundary(BaseTest):
    """A delegation set in one chunk and cleared in the next must still
    chunk-prove: the clearing block's witness needs the old designator."""

    BATCH_SEALING_BLOCK_COUNT = 3

    def __init__(self, ctx: flexitest.InitContext):
        # Private env: chunk_sealing_block_count=1 turns every block into its
        # own chunk, so adjacent blocks are always separate chunks.
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                batch_sealing_block_count=self.BATCH_SEALING_BLOCK_COUNT,
                chunk_sealing_block_count=1,
            )
        )

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        rpc = alpen_seq.create_rpc()
        btc_rpc = bitcoin.create_rpc()
        miner_addr = btc_rpc.proxy.getnewaddress()
        log_path = Path(alpen_seq.props["datadir"]) / "service.log"

        # --- Stage 1: land (set, clear) pairs in adjacent blocks ---
        clear_block_hashes: list[str] = []
        for attempt in range(1, MAX_PAIR_ATTEMPTS + 1):
            block_hash = self._run_set_clear_pair(rpc, attempt)
            if block_hash is not None:
                clear_block_hashes.append(block_hash)
                if len(clear_block_hashes) >= ADJACENT_PAIRS_TARGET:
                    break

        assert clear_block_hashes, (
            f"no (set, clear) pair landed in adjacent blocks in {MAX_PAIR_ATTEMPTS} attempts; "
            "cannot exercise the cross-chunk shape"
        )
        logger.info(
            f"{len(clear_block_hashes)} adjacent set/clear pair(s) landed; "
            f"clearing blocks: {clear_block_hashes}"
        )

        # --- Stage 2: every clearing block's chunk must reach proof-ready ---
        #
        # Mining bitcoin blocks between polls advances DA confirmations, which
        # drives the batch lifecycle into ProofPending and triggers the chunk
        # prover. Chunk size is 1, so the chunk covering a clearing block is
        # exactly the chunk whose `last_block` is that block's hash (matched
        # against the ChunkId debug output in the proof-ready log line).
        for block_hash in clear_block_hashes:
            self._wait_chunk_proof_ready(log_path, block_hash, btc_rpc, miner_addr)

        # A panic on any other chunk is the same regression surfacing in a
        # placement we didn't track — never acceptable.
        body = _read_log(log_path)
        assert PANIC_SIGNATURE not in body, (
            f"chunk prover panicked with {PANIC_SIGNATURE!r} (log: {log_path})"
        )
        assert "retries exhausted" not in body, (
            f"prover task(s) permanently failed (log: {log_path})"
        )

        logger.info("all clearing-block chunks proof-ready; no prover panics")
        return True

    def _run_set_clear_pair(self, rpc, attempt: int) -> str | None:
        """Set then clear the authority's delegation in consecutive blocks.

        Returns the clearing block's hash when the two txs landed in adjacent
        blocks (the shape that puts the old designator's only witness copy in
        the previous chunk), else None.
        """
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        auth_nonce = int(rpc.eth_getTransactionCount(AUTHORITY_ADDRESS, "latest"), 16)
        gas_price = int(rpc.eth_gasPrice(), 16)

        # Pre-sign both legs so the clear can be broadcast the instant the
        # set's receipt appears. The applied set bumps the authority nonce by
        # exactly one, so the clear authorization's nonce is deterministic.
        raw_set = _sign_delegation_tx(
            dev_nonce=dev_nonce,
            auth_nonce=auth_nonce,
            target=DELEGATE_TARGET,
            gas_price=gas_price,
        )
        raw_clear = _sign_delegation_tx(
            dev_nonce=dev_nonce + 1,
            auth_nonce=auth_nonce + 1,
            target=ZERO_ADDRESS,
            gas_price=gas_price,
        )

        set_receipt = _wait_receipt_fast(rpc, rpc.eth_sendRawTransaction(raw_set))
        assert set_receipt["status"] == "0x1", f"set tx failed: {set_receipt}"
        designator = "0xef0100" + DELEGATE_TARGET[2:].lower()
        code = rpc.eth_getCode(AUTHORITY_ADDRESS, "latest")
        if code.lower() != designator:
            logger.info(f"attempt {attempt}: set authorization not applied (code={code}); retrying")
            return None

        clear_receipt = _wait_receipt_fast(rpc, rpc.eth_sendRawTransaction(raw_clear))
        assert clear_receipt["status"] == "0x1", f"clear tx failed: {clear_receipt}"
        code = rpc.eth_getCode(AUTHORITY_ADDRESS, "latest")
        if code not in ("0x", "0x0"):
            logger.info(f"attempt {attempt}: clear not applied (code={code}); retrying")
            return None

        set_block = int(set_receipt["blockNumber"], 16)
        clear_block = int(clear_receipt["blockNumber"], 16)
        if clear_block != set_block + 1:
            logger.info(
                f"attempt {attempt}: set in block {set_block}, clear in block {clear_block} "
                "(not adjacent); retrying"
            )
            return None

        logger.info(
            f"attempt {attempt}: adjacent pair — set in block {set_block}, "
            f"clear in block {clear_block} ({clear_receipt['blockHash']})"
        )
        return clear_receipt["blockHash"]

    def _wait_chunk_proof_ready(
        self, log_path: Path, block_hash: str, btc_rpc, miner_addr: str
    ) -> None:
        """Wait for "marking chunk as proof-ready" naming `block_hash` as the
        chunk's last block; fail fast on the missing-bytecode panic."""
        hash_hex = block_hash.removeprefix("0x").lower()
        # The line carries the hash twice: full hex in the ChunkId debug output
        # (`last_block: <64 hex>`) and truncated in the proof_id display
        # (`abcdef..abcdef`, first/last 3 bytes). Match either, so the test
        # survives formatting drift in one of them.
        truncated = re.escape(f"{hash_hex[:6]}..{hash_hex[-6:]}")
        proof_ready = re.compile(
            rf"marking chunk as proof-ready.*(?:{hash_hex}|{truncated})",
            re.IGNORECASE,
        )

        def poll() -> bool:
            body = _read_log(log_path)
            if PANIC_SIGNATURE in body:
                raise AssertionError(
                    f"chunk prover panicked with {PANIC_SIGNATURE!r}: the clearing block's "
                    "witness lacks the pre-state delegation designator (cross-chunk 7702 "
                    f"capture regression). Log: {log_path}"
                )
            if proof_ready.search(body):
                return True
            # Advance DA confirmations so the batch lifecycle reaches the prover.
            btc_rpc.proxy.generatetoaddress(4, miner_addr)
            return False

        wait_until(
            poll,
            error_with=(
                f"chunk with last_block {block_hash} never reached proof-ready within "
                f"{CHUNK_PROOF_TIMEOUT_SECS}s (log: {log_path})"
            ),
            timeout=CHUNK_PROOF_TIMEOUT_SECS,
            step=1.0,
        )
        logger.info(f"chunk with last_block {block_hash} is proof-ready")

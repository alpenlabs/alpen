"""End-to-end EE chunk + acct prover pipeline check.

Drives a non-trivial *mix* of EVM activity through the EE — plain ETH
transfers, storage-writing contract deployments, and large-runtime
contract deployments — then verifies (a) every transaction's on-chain
effect is correct and (b) the produced blocks roll through chunk-seal,
chunk proof, acct proof, and OL submission in native dev-prover mode.

The mix matters: it exercises different code paths in the chunk witness
extraction:
  - transfers → balance/nonce reads + writes
  - storage_filler → SSTORE chains with distinct slot keys
  - large_runtime → distinct bytecode hashes flowing through the
    content-addressed `BytecodeSchema` (phase 2 redesign)

Native mode runs the full guest program logic in plain Rust; only the
zk proof generation itself is bypassed. So this test exercises:
  - Inline per-block witness production: during payload build,
    `build_block_witness_from_executed_state` harvests the depth-0 witness
    parts straight from the just-executed reth `State` (no re-execution) into a
    `BlockWitnessRecord`, carried on the payload and persisted to the
    `BlockWitnessStore`. A capture failure fails the payload build, so this
    gates block acceptance.
  - `ChunkSpec::fetch_input` (sled reads of the per-block `BlockWitnessRecord`s,
    unioned into one chunk-level sparse state via
    `EvmPartialState::from_witness_parts` / rsp `from_execution_witness`)
  - `AccessedStateGenerator` exex (per-block accessed-state writes, feeding
    the account proof's batch-range witness)
  - The `EeChunkProgram` guest (per-block state transition checks, MPT validation)
  - The `EeAcctProgram` guest (update aggregation, pub-params construction)
  - The paas service framework (task lifecycle, receipt hooks, OL submission)

The test asserts via the service.log signals the rest of the codebase
already produces — no storage poking, no new RPC surface. Each
assertion is independent so a regression at any single stage pinpoints
the failure.
N.B. The test is intended to be short-lived and parts to be substituted
with proper accessors or state asserts (and not logs).
"""

import logging
import re
from pathlib import Path

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.evm import (
    DEV_ACCOUNT_ADDRESS,
    deploy_large_runtime_contract,
    deploy_storage_filler,
    send_eth_transfer,
)
from common.evm_utils import wait_for_receipt
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.wait import wait_until_with_value
from envconfigs.el_ol import EeOLEnv

logger = logging.getLogger(__name__)

# --- EVM activity knobs ---
#
# Mix is biased toward diversity rather than raw transaction count: each
# category exercises a different aspect of the chunk witness.
TRANSFER_COUNT = 20  # plain ETH transfers (balance/nonce slots)
STORAGE_CONTRACT_COUNT = 5  # deploys that SSTORE to multiple slots
SLOTS_PER_STORAGE_CONTRACT = 8
LARGE_CONTRACT_COUNT = 5  # deploys with large identical runtime
LARGE_RUNTIME_SIZE = 5_000  # bytes; identical across all 5 → bytecode dedup

TRANSFER_AMOUNT_WEI = 10**16  # 0.01 ETH per transfer
TRANSFER_RECIPIENT = "0x000000000000000000000000000000000000dEaD"

# Per-signal timeout. Each prover-pipeline post-condition gets its own
# deadline so failures pinpoint which stage of the pipeline stalled.
SIGNAL_TIMEOUT_SECS = 180

# Service logs include tracing ANSI colour codes even when written to file.
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def _ee_log_path(alpen_service: AlpenClientService) -> Path:
    """Path to alpen-client's service log produced by the test harness."""
    return Path(alpen_service.props["datadir"]) / "service.log"


def _count_log_matches(log_path: Path, pattern: str, after_offset: int = 0) -> int:
    """Return the number of `pattern` matches in `log_path` past `after_offset`.

    Tolerates a not-yet-created log file (returns 0).
    """
    if not log_path.exists():
        return 0
    with log_path.open("rb") as fh:
        fh.seek(after_offset)
        body = fh.read().decode(errors="replace")
    body = _ANSI_RE.sub("", body)
    return sum(1 for _ in re.finditer(pattern, body))


def _wait_for_log_signal(
    log_path: Path,
    pattern: str,
    after_offset: int,
    timeout: int,
    description: str,
    btc_rpc,
    miner_addr: str,
    btc_blocks_per_step: int = 4,
    poll: float = 1.0,
) -> int:
    """Poll until at least one match for `pattern` appears past `after_offset`.

    Mines bitcoin blocks between polls so the batch DA confirmations
    advance, which is what eventually drives the batch lifecycle into
    `ProofPending` and triggers the chunk + acct prover request.
    """

    def mine_and_count() -> int:
        count = _count_log_matches(log_path, pattern, after_offset)
        if count == 0:
            btc_rpc.proxy.generatetoaddress(btc_blocks_per_step, miner_addr)
        return count

    count = wait_until_with_value(
        mine_and_count,
        lambda c: c > 0,
        error_with=(
            f"{description}: no log match for {pattern!r} within {timeout}s (log: {log_path})"
        ),
        timeout=timeout,
        step=poll,
    )
    logger.info(f"{description}: observed {count} match(es)")
    return count


@flexitest.register
class TestEeProverPipelineAlive(BaseTest):
    """Verify the EE chunk + acct prover pipeline runs end-to-end under
    realistic, varied EVM transaction load, in native dev-prover mode."""

    # Tighter than the shared `el_ol` env's default (10): smaller batches
    # mean more chunk/acct proofs per unit of EVM activity, which keeps
    # the test fast while still exercising the full pipeline multiple times.
    BATCH_SEALING_BLOCK_COUNT = 3

    def __init__(self, ctx: flexitest.InitContext):
        # Inline env instance — flexitest gives this test its own private
        # alpen-client + strata + bitcoin trio rather than reusing a
        # shared instance. We need the log file to ourselves so we can
        # assert "this signal fired during this test" without inheriting
        # log lines from sibling tests that ran on the same env earlier.
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                batch_sealing_block_count=self.BATCH_SEALING_BLOCK_COUNT,
            )
        )

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        rpc = alpen_seq.create_rpc()
        btc_rpc = bitcoin.create_rpc()
        miner_addr = btc_rpc.proxy.getnewaddress()
        log_path = _ee_log_path(alpen_seq)

        # --- Stage 1: submit a mix of EVM activity ---
        #
        # All transactions come from the dev account so nonces are a
        # single monotonic sequence — easier than juggling multiple
        # senders.

        start_nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        nonce = start_nonce

        recipient_balance_before = int(rpc.eth_getBalance(TRANSFER_RECIPIENT, "latest"), 16)

        # (a) Plain ETH transfers.
        for _ in range(TRANSFER_COUNT):
            send_eth_transfer(rpc, nonce, TRANSFER_RECIPIENT, TRANSFER_AMOUNT_WEI)
            nonce += 1

        # (b) Storage-filler deploys. Each SSTOREs to N distinct slots,
        # producing varied multiproof targets per chunk.
        storage_tx_hashes = []
        for _ in range(STORAGE_CONTRACT_COUNT):
            storage_tx_hashes.append(deploy_storage_filler(rpc, nonce, SLOTS_PER_STORAGE_CONTRACT))
            nonce += 1

        # (c) Large-runtime deploys, all identical size so they share a
        # code hash — exercises the content-addressed bytecode cache
        # (phase 2 of the redesign).
        large_tx_hashes = []
        for _ in range(LARGE_CONTRACT_COUNT):
            large_tx_hashes.append(
                deploy_large_runtime_contract(rpc, nonce, runtime_size=LARGE_RUNTIME_SIZE)
            )
            nonce += 1

        total_tx = TRANSFER_COUNT + STORAGE_CONTRACT_COUNT + LARGE_CONTRACT_COUNT
        logger.info(
            f"submitted {total_tx} txs: {TRANSFER_COUNT} transfers, "
            f"{STORAGE_CONTRACT_COUNT} storage contracts, "
            f"{LARGE_CONTRACT_COUNT} large-runtime contracts"
        )

        # --- Stage 2: validate every tx's on-chain effect ---
        #
        # Sequential nonces from a single sender → waiting on the final
        # deploy's receipt guarantees all earlier transactions landed.

        last_tx_hash = large_tx_hashes[-1]
        final_receipt = wait_for_receipt(rpc, last_tx_hash, timeout=120)
        assert final_receipt["status"] == "0x1", f"final tx failed: {final_receipt}"
        final_block = int(final_receipt["blockNumber"], 16)
        logger.info(f"all submitted txs accepted by block {final_block}")

        # Anchor the log baseline HERE — after the final tx receipt — not
        # at the top of main. Reasons:
        #   * Pre-test alpen-client startup may have already produced log
        #     entries that match our patterns (batches can seal during the
        #     env's pre_generate_blocks warm-up). Counting those would
        #     give a false-positive "pipeline alive" without our txs ever
        #     reaching the prover.
        #   * Batches seal in monotonic block-number order, so any prover
        #     signal past this offset is necessarily for a batch whose
        #     last block ≥ final_block — i.e. attributable to our load.
        # Each subsequent `_wait_for_log_signal` re-reads from this same
        # offset to EOF on every poll iteration; concurrent log writes
        # are safe (single-line regex match, partial tails just retry).
        log_offset = log_path.stat().st_size if log_path.exists() else 0
        logger.info(
            f"log baseline anchored at offset={log_offset} after final tx block={final_block}"
        )

        # (a) Transfer recipient balance moved by exactly the expected amount.
        recipient_balance_after = int(rpc.eth_getBalance(TRANSFER_RECIPIENT, "latest"), 16)
        expected_balance = recipient_balance_before + TRANSFER_COUNT * TRANSFER_AMOUNT_WEI
        assert recipient_balance_after == expected_balance, (
            f"recipient balance: got {recipient_balance_after}, expected {expected_balance}"
        )
        logger.info(
            f"transfer recipient balance advanced by {TRANSFER_COUNT * TRANSFER_AMOUNT_WEI} wei"
        )

        # (b) Each storage contract was deployed and wrote slots 0..N-1
        # with values 1..N (per deploy_storage_filler's init code).
        for tx_hash in storage_tx_hashes:
            r = wait_for_receipt(rpc, tx_hash, timeout=60)
            assert r["status"] == "0x1", f"storage deploy failed: {r}"
            addr = r["contractAddress"]
            assert addr, f"no contractAddress in receipt: {r}"
            for slot in range(SLOTS_PER_STORAGE_CONTRACT):
                slot_key = "0x" + format(slot, "064x")
                val_hex = rpc.eth_getStorageAt(addr, slot_key, "latest")
                val = int(val_hex, 16)
                assert val == slot + 1, (
                    f"contract {addr} slot {slot}: got {val}, expected {slot + 1}"
                )
        logger.info(
            f"verified storage on {STORAGE_CONTRACT_COUNT} contracts "
            f"× {SLOTS_PER_STORAGE_CONTRACT} slots each"
        )

        # (c) Each large-runtime contract has runtime code of the
        # configured size (and identical code hashes across deploys —
        # if any deviated, bytecode cache wouldn't dedup).
        first_code = None
        for tx_hash in large_tx_hashes:
            r = wait_for_receipt(rpc, tx_hash, timeout=60)
            assert r["status"] == "0x1", f"large-runtime deploy failed: {r}"
            addr = r["contractAddress"]
            assert addr, f"no contractAddress in receipt: {r}"
            code = rpc.eth_getCode(addr, "latest")
            # "0x" + 2 hex chars per byte.
            assert len(code) == 2 + LARGE_RUNTIME_SIZE * 2, (
                f"contract {addr} runtime size: got {(len(code) - 2) // 2} bytes, "
                f"expected {LARGE_RUNTIME_SIZE}"
            )
            if first_code is None:
                first_code = code
            else:
                assert code == first_code, (
                    "large-runtime contracts must share identical bytecode for "
                    "dedup test; saw different code at some address"
                )
        logger.info(
            f"verified {LARGE_CONTRACT_COUNT} large-runtime contracts share "
            f"the same {LARGE_RUNTIME_SIZE}-byte runtime"
        )

        # --- Stage 3: walk the four prover-pipeline stages ---
        #
        # Each signal must appear in the post-baseline log fragment.
        # Polling drives bitcoin block production so the batch DA window
        # advances, which is what eventually triggers proof requests.

        _wait_for_log_signal(
            log_path,
            r"persisted block witness",
            after_offset=log_offset,
            timeout=SIGNAL_TIMEOUT_SECS,
            description="per-block witness persisted inline at block production",
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )

        _wait_for_log_signal(
            log_path,
            r"marking chunk as proof-ready",
            after_offset=log_offset,
            timeout=SIGNAL_TIMEOUT_SECS,
            description="chunk proof completed (ChunkReceiptHook fired)",
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )

        _wait_for_log_signal(
            log_path,
            r"persisting batch acct proof",
            after_offset=log_offset,
            timeout=SIGNAL_TIMEOUT_SECS,
            description="acct proof completed (AcctReceiptHook fired)",
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )

        _wait_for_log_signal(
            log_path,
            r"submitted snark update to OL",
            after_offset=log_offset,
            timeout=SIGNAL_TIMEOUT_SECS,
            description="acct proof submitted to OL (SnarkAccountUpdate)",
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )

        # Negative check: no permanent failure in the post-baseline window.
        perm_fail_count = _count_log_matches(
            log_path,
            r"retries exhausted|task died mid-Proving and retries exhausted",
            after_offset=log_offset,
        )
        assert perm_fail_count == 0, (
            f"observed {perm_fail_count} permanent prover failure(s) in log fragment "
            f"(log: {log_path})"
        )

        logger.info(
            "EE prover pipeline alive: %d varied EVM txs validated on-chain, "
            "chunk + acct proofs landed end-to-end",
            total_tx,
        )
        return True

"""Regression: EIP-7702 delegation cleared across a *batch* boundary.

Repro for the acct/outer-proof panic
``DA witness verification failed: PostApplyStateRootMismatch``:

  batch A:  an EIP-7702 tx *sets* a delegation on authority X (code_hash: empty
            -> designator)
  batch B:  a later EIP-7702 tx *clears* it (code_hash: designator -> empty)

The DA state diff is a *batch*-level diff: it compares each account's state at
the batch's start against its end. ``AccountDiff::from_account_snapshot``
(``crates/reth/statediff/src/batch/account.rs``) drops the code_hash change
whenever the *current* value is ``KECCAK_EMPTY`` — a stale "code only changes on
contract creation" assumption that predates EIP-7702. So batch B's blob records
no code_hash change for X, and DA reconstruction keeps X's old designator hash
while the proven chunk execution cleared it. The outer proof's
``verify_da_witness`` then finds the reassembled root diverges from the last
chunk's ``tip_state_root`` and panics.

Unlike the chunk-prover repro (which turns on chunk *layout*), this bug is
purely batch-level: chunk boundaries are irrelevant. ``batch_sealing_block_count
= 1`` makes every EE block its own batch, so the set and the clear — landing in
different blocks (the clear is broadcast only after the set is mined) — always
fall in different batches. The test then drives the batch lifecycle until the
clearing batch's acct proof runs: on a fixed node it is persisted; on an
unfixed node it panics with the mismatch signature and the test fails fast.
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

# The 7702 authority whose delegation is set then cleared. It never sends its
# own transactions (the dev account sponsors both legs), so it needs no funds;
# its nonce advances only through applied authorizations.
AUTHORITY_PRIVATE_KEY = "0x" + "cd" * 32
AUTHORITY_ADDRESS = Account.from_key(AUTHORITY_PRIVATE_KEY).address

# Set-leg delegation target. Any address works: the designator stored on the
# authority is `0xef0100 || target` whether or not code exists there.
DELEGATE_TARGET = "0x00000000000000000000000000000000000000fe"
# Per EIP-7702, delegating to the zero address clears the designator.
ZERO_ADDRESS = "0x" + "00" * 20

RECEIPT_TIMEOUT_SECS = 30
# The acct/outer proof runs behind the whole lifecycle (DA post + confirm +
# chunk proof + acct proof) for every preceding single-block batch, so allow
# generous headroom over the chunk-only repro.
ACCT_PROOF_TIMEOUT_SECS = 360

# Outer-proof panic surfaced through paas's "prove task panicked" log line on an
# unfixed node.
MISMATCH_SIGNATURE = "PostApplyStateRootMismatch"
DA_VERIFY_FAIL_SIGNATURE = "DA witness verification failed"

# Emitted by AcctReceiptHook once the clearing batch's outer proof is stored —
# the success signal that the mismatch never fired.
ACCT_PROOF_STORED = "persisting batch acct proof"

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


def _wait_receipt(rpc, tx_hash: str) -> dict:
    deadline = time.monotonic() + RECEIPT_TIMEOUT_SECS
    while time.monotonic() < deadline:
        receipt = rpc.eth_getTransactionReceipt(tx_hash)
        if receipt is not None:
            return receipt
        time.sleep(0.1)
    raise AssertionError(f"no receipt for {tx_hash} within {RECEIPT_TIMEOUT_SECS}s")


def _read_log(log_path: Path) -> str:
    if not log_path.exists():
        return ""
    return _ANSI_RE.sub("", log_path.read_bytes().decode(errors="replace"))


@flexitest.register
class TestEeProver7702DaBatchBoundary(BaseTest):
    """A delegation set in one batch and cleared in a later batch must still
    outer-prove: the clearing batch's DA blob must record the code_hash clear."""

    def __init__(self, ctx: flexitest.InitContext):
        # batch_sealing_block_count=1: every EE block is its own batch, so the
        # set and the clear always land in separate batches. chunk=1 keeps one
        # chunk per batch (chunk layout is irrelevant to this bug).
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                batch_sealing_block_count=1,
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

        # --- Stage 1: set then clear the delegation in different batches ---
        clear_block_hash = self._set_then_clear(rpc)

        # --- Stage 2: the clearing batch's acct proof must be persisted ---
        # Mining bitcoin blocks advances DA confirmations, driving each batch
        # through DaComplete -> ProofPending -> (chunk + acct proofs) ->
        # ProofReady. batch_sealing=1 makes the clearing batch's `last_block`
        # exactly `clear_block_hash`, so its acct-proof log line carries it.
        self._wait_acct_proof(log_path, clear_block_hash, btc_rpc, miner_addr)

        logger.info("clearing batch outer-proved; no DA state-root mismatch")
        return True

    def _set_then_clear(self, rpc) -> str:
        """Set the authority's delegation, then clear it in a later block.

        Returns the clearing block's hash. With ``batch_sealing_block_count=1``
        this block is its own batch, whose start-state still holds the
        designator (set in an earlier batch) and end-state is empty — the shape
        that drops the code_hash change from the batch DA blob.
        """
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        auth_nonce = int(rpc.eth_getTransactionCount(AUTHORITY_ADDRESS, "latest"), 16)
        gas_price = int(rpc.eth_gasPrice(), 16)

        # Pre-sign both legs so the clear can be broadcast the instant the set's
        # receipt appears. The applied set bumps the authority nonce by one, so
        # the clear authorization's nonce is deterministic.
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

        set_receipt = _wait_receipt(rpc, rpc.eth_sendRawTransaction(raw_set))
        assert set_receipt["status"] == "0x1", f"set tx failed: {set_receipt}"
        designator = "0xef0100" + DELEGATE_TARGET[2:].lower()
        code = rpc.eth_getCode(AUTHORITY_ADDRESS, "latest")
        assert code.lower() == designator, f"set authorization not applied (code={code})"

        clear_receipt = _wait_receipt(rpc, rpc.eth_sendRawTransaction(raw_clear))
        assert clear_receipt["status"] == "0x1", f"clear tx failed: {clear_receipt}"
        code = rpc.eth_getCode(AUTHORITY_ADDRESS, "latest")
        assert code in ("0x", "0x0"), f"clear not applied (code={code})"

        set_block = int(set_receipt["blockNumber"], 16)
        clear_block = int(clear_receipt["blockNumber"], 16)
        # The clear is broadcast only after the set is mined, so it lands in a
        # strictly later block — and thus a later batch (batch_sealing=1).
        assert clear_block > set_block, (
            f"clear (block {clear_block}) not after set (block {set_block}); "
            "cannot exercise the cross-batch shape"
        )
        logger.info(
            f"set in block {set_block} (batch), cleared in block {clear_block} "
            f"({clear_receipt['blockHash']})"
        )
        return clear_receipt["blockHash"]

    def _wait_acct_proof(self, log_path: Path, block_hash: str, btc_rpc, miner_addr: str) -> None:
        """Wait for the clearing batch's acct proof to be persisted; fail fast on
        the DA state-root mismatch panic."""
        hash_hex = block_hash.removeprefix("0x").lower()
        # BatchId renders as `<prev_hex>:<last_hex>` with full hashes. Anchor on
        # the `:last_block` half so this matches the clearing batch's own line,
        # not the next batch's (which carries the same hash as its prev_block).
        proof_stored = re.compile(
            rf"{re.escape(ACCT_PROOF_STORED)}.*:{hash_hex}",
            re.IGNORECASE,
        )

        def poll() -> bool:
            body = _read_log(log_path)
            if MISMATCH_SIGNATURE in body or DA_VERIFY_FAIL_SIGNATURE in body:
                raise AssertionError(
                    f"acct proof panicked with {MISMATCH_SIGNATURE!r}: the clearing batch's "
                    "DA blob dropped the 7702 delegation code_hash clear "
                    "(designator -> empty), so reconstruction diverges from the proven "
                    f"tip_state_root. Log: {log_path}"
                )
            if "retries exhausted" in body:
                raise AssertionError(f"prover task(s) permanently failed (log: {log_path})")
            if proof_stored.search(body):
                return True
            # Advance DA confirmations so the batch lifecycle reaches the prover.
            btc_rpc.proxy.generatetoaddress(4, miner_addr)
            return False

        wait_until(
            poll,
            error_with=(
                f"clearing batch (last_block {block_hash}) acct proof was not persisted "
                f"within {ACCT_PROOF_TIMEOUT_SECS}s (log: {log_path})"
            ),
            timeout=ACCT_PROOF_TIMEOUT_SECS,
            step=1.0,
        )
        logger.info(f"clearing batch (last_block {block_hash}) acct proof persisted")

"""
Real-bridge deposit/withdrawal helpers for functional tests.

Wraps `strata-test-cli compute-drt-output` (DRT output spec) and
`strata-test-cli create-deposit-tx` (DT signing) and constructs the actual
on-chain transactions via bitcoind RPC. Use this when testing the real
bridge subprotocol path; for the lighter debug-subprotocol path, use
`common.test_cli.create_mock_deposit` instead.
"""

import json
import logging
import os
import re
import time
from dataclasses import dataclass
from pathlib import Path

from eth_account import Account
from eth_keys import keys

from common.config.constants import SATS_TO_WEI
from common.precompile import PRECOMPILE_BRIDGEOUT_ADDRESS, wait_for_receipt
from common.services.alpen_client import AlpenClientService
from common.test_cli import _run_command

logger = logging.getLogger(__name__)


# Genesis registers the Alpen EE as the first user account, which lands at
# serial 128 (system serials occupy 0..128).
ALPEN_EE_ACCOUNT_SERIAL = 128

# DT input value (DRT output 1) must exceed the configured bridge denomination
# so the DT has a positive mining fee. The bridge subprotocol validates DT
# output 1 against that denomination; the difference is the miner fee. 1000 sats
# covers a small Schnorr-witness DT comfortably under any sane regtest mempool
# min relay rate.
DT_FEE_BUFFER_SATS = 1_000
BOSD_P2WPKH_TAG = "03"

# Bridgeout calldata: [4 bytes: selected_operator (big-endian u32)][BOSD bytes].
# 0xFFFFFFFF = u32::MAX = "no specific operator, bridge picks".
NO_OPERATOR_SELECTION_HEX = "ffffffff"


@dataclass
class DrtOutput:
    """Result of `strata-test-cli compute-drt-output`."""

    bridge_in_address: str
    op_return_hex: str
    amount_sats: int

    @property
    def op_return_data_hex(self) -> str:
        """Strip the OP_RETURN opcode + push-byte prefix to get the bare data.

        For SPS-50 DRT payloads (~60 bytes), bitcoind's `createrawtransaction`
        wants the data portion only and wraps it in OP_RETURN itself. The
        layout is `0x6a <push_op> <data>`. For data <= 75 bytes, `push_op` is
        a single direct-push byte (0x01-0x4b) and stripping `op_return_hex[4:]`
        gives the data. For larger payloads bitcoin uses `OP_PUSHDATA1`
        (0x4c) followed by a length byte (or `OP_PUSHDATA2`/`4` for even
        larger), in which case the offset is no longer 2 hex chars. We
        explicitly reject anything outside the direct-push range so we don't
        silently corrupt the data.
        """
        if len(self.op_return_hex) < 4:
            raise ValueError(f"op_return_hex too short: {self.op_return_hex!r}")
        if self.op_return_hex[:2] != "6a":
            raise ValueError(f"expected OP_RETURN (0x6a) prefix, got {self.op_return_hex[:2]!r}")
        push_op = int(self.op_return_hex[2:4], 16)
        # Direct-push opcodes 0x01..0x4b push that many bytes directly.
        # 0x00 is OP_0 (no data), 0x4c+ are OP_PUSHDATA1/2/4 with extra length bytes.
        if push_op == 0 or push_op > 0x4B:
            raise ValueError(
                f"unexpected push opcode 0x{push_op:02x} in op_return_hex; "
                "this stripper only handles direct-push payloads (1..75 bytes). "
                "Use OP_PUSHDATA1+ aware logic for larger payloads."
            )
        # Sanity: push_op should equal the number of data bytes that follow.
        actual_data_len = (len(self.op_return_hex) - 4) // 2
        if push_op != actual_data_len:
            raise ValueError(
                f"push opcode 0x{push_op:02x} ({push_op} bytes) doesn't match "
                f"actual data length {actual_data_len}"
            )
        return self.op_return_hex[4:]


def random_xonly_pubkey_hex() -> str:
    """Generate a fresh, valid x-only secp256k1 pubkey for the DRT recovery path.

    A random 32-byte string is *not* guaranteed to be a valid x-only pubkey
    (the x-coord must lie on secp256k1; about half of all 32-byte values
    don't). Instead we derive one from a fresh private key and take the
    pubkey's x coordinate. The recovery path is never exercised on the happy
    path, but the script-builder still parses this value as an x-only pubkey
    when constructing the takeback tapleaf, so it must be valid.
    """
    priv_bytes = os.urandom(32)
    pub_uncompressed = keys.PrivateKey(priv_bytes).public_key.to_bytes()  # 64 bytes: x|y
    return pub_uncompressed[:32].hex()


def derive_p2wpkh_bosd_hex(btc_rpc) -> str:
    """Derive a fresh P2WPKH bridge output script descriptor payload."""
    addr = btc_rpc.proxy.getnewaddress("", "bech32")
    info = btc_rpc.proxy.getaddressinfo(addr)
    spk = info["scriptPubKey"]
    if not spk.startswith("0014") or len(spk) != 4 + 40:
        raise RuntimeError(f"unexpected scriptPubKey for P2WPKH {addr}: {spk}")
    return BOSD_P2WPKH_TAG + spk[4:]


def rpc_quantity_to_int(value) -> int:
    """Convert an Ethereum JSON-RPC quantity to int."""
    return int(value, 16) if isinstance(value, str) else int(value)


def ee_log_path(alpen_service: AlpenClientService) -> Path:
    """Path to the alpen-client service log produced by the test harness."""
    return Path(alpen_service.props["datadir"]) / "service.log"


# Service logs can contain ANSI colour codes even when written to files.
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def _strip_ansi(text: str) -> str:
    return _ANSI_RE.sub("", text)


def wait_for_output_snark_update(
    log_path: Path,
    btc_rpc,
    miner_addr: str,
    after_offset: int,
    timeout: int = 600,
    blocks_per_step: int = 4,
    poll: float = 1.0,
) -> int:
    """Wait for alpen-client to submit the SAU carrying a withdrawal output."""
    pattern = re.compile(
        r"submitted snark update to OL\b.*seq_no=(\d+).*output_message_count=([1-9]\d*)"
    )
    deadline = time.time() + timeout
    while time.time() < deadline:
        if log_path.exists():
            with open(log_path, "rb") as f:
                f.seek(after_offset)
                tail = _strip_ansi(f.read().decode(errors="replace"))
            match = pattern.search(tail)
            if match:
                return int(match.group(1))
        btc_rpc.proxy.generatetoaddress(blocks_per_step, miner_addr)
        time.sleep(poll)
    raise AssertionError(
        f"no alpen-client output SnarkAccountUpdate in {log_path} within {timeout}s"
    )


def submit_bridgeout_transaction(
    alpen_rpc,
    from_address: str,
    from_private_key_hex: str,
    recipient_bosd_hex: str,
    withdraw_sats: int,
) -> str:
    """Submit a bridgeout precompile transaction from an EE account."""
    chain_id = int(alpen_rpc.eth_chainId(), 16)
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    nonce = int(alpen_rpc.eth_getTransactionCount(from_address, "latest"), 16)

    withdraw_tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": 200_000,
        "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
        "value": withdraw_sats * SATS_TO_WEI,
        "data": bytes.fromhex(NO_OPERATOR_SELECTION_HEX + recipient_bosd_hex),
        "chainId": chain_id,
    }
    signed = Account.sign_transaction(withdraw_tx, from_private_key_hex)
    return alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())


def assert_bridgeout_receipt(alpen_rpc, tx_hash: str, timeout: int = 30) -> int:
    """Wait for a bridgeout receipt and return the gas spent in wei."""
    receipt = wait_for_receipt(alpen_rpc, tx_hash, timeout=timeout)
    if receipt["status"] not in (1, "0x1"):
        raise AssertionError(f"bridgeout call reverted: {receipt}")
    if not receipt["logs"]:
        raise AssertionError("bridgeout did not emit WithdrawalIntentEvent")

    gas_used = rpc_quantity_to_int(receipt["gasUsed"])
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    effective_gas_price = rpc_quantity_to_int(receipt.get("effectiveGasPrice", gas_price))
    return gas_used * effective_gas_price


def compute_drt_output(
    operator_xprivs_hex: list[str],
    recovery_pubkey_hex: str,
    alpen_address_hex: str,
    account_serial: int = ALPEN_EE_ACCOUNT_SERIAL,
    network: str = "regtest",
) -> DrtOutput:
    """Run `strata-test-cli compute-drt-output` and parse the JSON result."""
    args = [
        "compute-drt-output",
        "--operator-keys",
        json.dumps(operator_xprivs_hex),
        "--recovery-pubkey",
        recovery_pubkey_hex,
        "--alpen-address",
        alpen_address_hex,
        "--account-serial",
        str(account_serial),
        "--network",
        network,
    ]
    out = _run_command(args)
    data = json.loads(out)
    return DrtOutput(
        bridge_in_address=data["bridge_in_address"],
        op_return_hex=data["op_return_hex"],
        amount_sats=data["amount_sats"],
    )


def broadcast_drt(
    btc_rpc,
    drt: DrtOutput,
    depositor_change_address: str,
) -> tuple[str, str]:
    """Build, sign, and broadcast the Deposit Request Transaction.

    Funds the DRT from the bitcoind wallet's UTXO set. The DRT must place the
    OP_RETURN at output index 0 and the P2TR `bridge_in` output at index 1,
    per the SPS-50 layout. We pin `changePosition=2` so any wallet-added
    change output sits after our two required outputs.

    Returns (txid, raw_tx_hex).
    """
    proxy = btc_rpc.proxy

    # Pad the bridge_in output with `DT_FEE_BUFFER_SATS` so the DT can spend it
    # with a positive fee. The bridge subprotocol validates DT output 1 against
    # the configured denomination, not the DRT input amount, so this padding is
    # invisible to ASM-side logic.
    bridge_in_sats_with_fee = drt.amount_sats + DT_FEE_BUFFER_SATS
    bridge_in_btc = bridge_in_sats_with_fee / 100_000_000

    # Output array order is preserved by bitcoind: [OP_RETURN, bridge_in P2TR].
    outputs = [
        {"data": drt.op_return_data_hex},
        {drt.bridge_in_address: bridge_in_btc},
    ]

    raw_tx = proxy.createrawtransaction([], outputs)
    funded = proxy.fundrawtransaction(
        raw_tx,
        {
            "changeAddress": depositor_change_address,
            "changePosition": 2,
        },
    )

    signed = proxy.signrawtransactionwithwallet(funded["hex"])
    if not signed.get("complete"):
        raise RuntimeError(f"DRT signing incomplete: {signed}")

    drt_hex = signed["hex"]
    drt_txid = proxy.sendrawtransaction(drt_hex)
    logger.info("DRT broadcast: txid=%s amount=%d sats", drt_txid, drt.amount_sats)
    return drt_txid, drt_hex


def create_and_broadcast_dt(
    btc_rpc,
    drt_hex: str,
    operator_xprivs_hex: list[str],
    dt_index: int,
) -> str:
    """Sign and broadcast the operator-side Deposit Transaction.

    Calls `strata-test-cli create-deposit-tx` with the DRT bytes and operator
    xprivs to produce a signed DT that consumes DRT output 1, then
    broadcasts via bitcoind.

    Returns the DT txid.
    """
    args = [
        "create-deposit-tx",
        "--drt-tx",
        drt_hex,
        "--operator-keys",
        json.dumps(operator_xprivs_hex),
        "--index",
        str(dt_index),
    ]
    dt_hex = _run_command(args)
    dt_txid = btc_rpc.proxy.sendrawtransaction(dt_hex)
    logger.info("DT broadcast: txid=%s dt_index=%d", dt_txid, dt_index)
    return dt_txid


def submit_real_bridge_deposit(
    btc_rpc,
    operator_xprivs_hex: list[str],
    alpen_address_hex: str,
    *,
    dt_index: int,
    account_serial: int = ALPEN_EE_ACCOUNT_SERIAL,
    network: str = "regtest",
) -> tuple[str, str, DrtOutput]:
    """End-to-end DRT + DT submission. Returns (drt_txid, dt_txid, drt_spec)."""
    proxy = btc_rpc.proxy
    recovery_pubkey_hex = random_xonly_pubkey_hex()

    drt = compute_drt_output(
        operator_xprivs_hex=operator_xprivs_hex,
        recovery_pubkey_hex=recovery_pubkey_hex,
        alpen_address_hex=alpen_address_hex,
        account_serial=account_serial,
        network=network,
    )

    depositor_change = proxy.getnewaddress()
    drt_txid, drt_hex = broadcast_drt(btc_rpc, drt, depositor_change)

    # Mine one block so the DRT is confirmed and visible to the DT signing path.
    miner_addr = proxy.getnewaddress()
    proxy.generatetoaddress(1, miner_addr)

    dt_txid = create_and_broadcast_dt(btc_rpc, drt_hex, operator_xprivs_hex, dt_index)
    return drt_txid, dt_txid, drt

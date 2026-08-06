"""
Real-bridge deposit/withdrawal helpers for functional tests.

Wraps `strata-test-cli compute-drt-output` (DRT output spec) and
`strata-test-cli create-deposit-tx` (DT signing) and constructs the actual
on-chain transactions via bitcoind RPC.

This is the only way to get a deposit onto the OL in tests. ASM v0.4.0-rc.1
removed the debug subprotocol that previously accepted synthetic deposit
logs, so every environment goes through the real bridge. The strata factory
seeds `bridge-operator_keys` into every datadir and datatool wires those
operators into the bridge subprotocol's genesis state, so these helpers work
in any strata env, not just the EE ones.
"""

import json
import logging
import os
from dataclasses import dataclass
from pathlib import Path

from eth_keys import keys

from common.services.strata import StrataService
from common.test_cli import _run_command

logger = logging.getLogger(__name__)


# Genesis registers the Alpen EE as the first user account, which lands at
# serial 128 (system serials occupy 0..128).
ALPEN_EE_ACCOUNT_SERIAL = 128

# Written by the strata factory at boot to seed the bridge subprotocol's
# genesis operator set.
OPERATOR_KEYS_FILENAME = "bridge-operator_keys"

# DT input value (DRT output 1) must exceed the configured bridge denomination
# so the DT has a positive mining fee. The bridge subprotocol validates DT
# output 1 against that denomination; the difference is the miner fee. 1000 sats
# covers a small Schnorr-witness DT comfortably under any sane regtest mempool
# min relay rate.
DT_FEE_BUFFER_SATS = 1_000


def _read_asm_params_int(strata_service: StrataService, key: str) -> int:
    """Recursively scan `asm-params.json` for the first integer field named `key`.

    Tolerates list-of-dict or flat-dict shapes so the schema can move
    Bridge fields under `subprotocols[N].Bridge.*` without breaking us.
    """
    datadir = Path(strata_service.props["datadir"])
    path = datadir / "asm-params.json"
    if not path.exists():
        raise RuntimeError(f"asm-params not found: {path}")
    raw = json.loads(path.read_text())

    def find(node) -> int | None:
        if isinstance(node, dict):
            if key in node:
                return int(node[key])
            for v in node.values():
                hit = find(v)
                if hit is not None:
                    return hit
        elif isinstance(node, list):
            for v in node:
                hit = find(v)
                if hit is not None:
                    return hit
        return None

    found = find(raw)
    if found is None:
        raise RuntimeError(f"`{key}` not found in {path}")
    return found


def read_operator_fee(strata_service: StrataService) -> int:
    """Bridge `operator_fee` (sats). Read from asm-params so the WF amount
    stays in sync with whatever datatool actually wrote."""
    return _read_asm_params_int(strata_service, "operator_fee")


def read_bridge_denomination(strata_service: StrataService) -> int:
    """Bridge `denomination` (sats). Read from asm-params for the same reason
    as `read_operator_fee`.

    Every deposit credits exactly this amount — the bridge validates the DT
    output against it — so tests that need a specific balance must deposit a
    multiple of it rather than picking an arbitrary figure.
    """
    return _read_asm_params_int(strata_service, "denomination")


def read_operator_xprivs(strata_service: StrataService) -> list[str]:
    """Read operator BIP32 xprivs (one per line) from the strata datadir.

    The strata factory writes this file at boot to seed the bridge
    subprotocol's genesis operator set; reading the same file keeps DT
    signing keys aligned with on-chain state.
    """
    datadir = Path(strata_service.props["datadir"])
    path = datadir / OPERATOR_KEYS_FILENAME
    if not path.exists():
        raise RuntimeError(f"operator key file not found: {path}")
    lines = [line.strip() for line in path.read_text().splitlines() if line.strip()]
    if not lines:
        raise RuntimeError(f"operator key file is empty: {path}")
    for i, line in enumerate(lines):
        if not (line.startswith("tprv") or line.startswith("xprv")):
            raise RuntimeError(
                f"line {i + 1} of {path} doesn't look like a BIP32 base58 xpriv: {line[:8]!r}..."
            )
    return lines


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

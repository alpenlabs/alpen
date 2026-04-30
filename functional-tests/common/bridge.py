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
import secrets
from dataclasses import dataclass

from common.test_cli import _run_command

logger = logging.getLogger(__name__)


# Genesis registers the Alpen EE as the first user account, which lands at
# serial 128 (system serials occupy 0..128).
ALPEN_EE_ACCOUNT_SERIAL = 128

# DT input value (DRT output 1) must exceed `denomination` so the DT has a
# positive mining fee. The bridge subprotocol enforces DT output 1 ==
# denomination (10 BTC); the difference is the miner fee. 1000 sats covers a
# small Schnorr-witness DT comfortably under any sane regtest mempool min
# relay rate.
DT_FEE_BUFFER_SATS = 1_000


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
        wants the data portion only and wraps it in OP_RETURN itself. The first
        two bytes of `op_return_hex` are 0x6a (OP_RETURN) and a single
        push-length byte; this stripping is correct for payloads <= 75 bytes.
        """
        if len(self.op_return_hex) < 4:
            raise ValueError(f"op_return_hex too short: {self.op_return_hex!r}")
        if self.op_return_hex[:2] != "6a":
            raise ValueError(f"expected OP_RETURN (0x6a) prefix, got {self.op_return_hex[:2]!r}")
        return self.op_return_hex[4:]


def random_xonly_pubkey_hex() -> str:
    """Generate a random 32-byte x-only pubkey for the DRT recovery path.

    The recovery path is never exercised in happy-path tests, so any byte
    string that parses as a valid x-only pubkey works. We pick a value
    deterministically via `secrets` so test runs vary, which is fine because
    the field is not consumed in the credit path.
    """
    return secrets.token_hex(32)


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

"""
End-to-end real-bridge deposit + withdrawal test.

Exercises the full deposit pipeline through the real bridge subprotocol
(no debug-subprotocol shortcut), then the full withdrawal pipeline back to
a user bitcoin wallet:

  1. Bitcoins on the OL snark account: build a real DRT/DT pair, broadcast
     them, and assert the Alpen EE snark account at serial 128 is credited.
  2. OL to EE: alpen-client consumes the inbox, EVM balance for the
     destination address increases.
  3. EE to OL: send to the bridgeout precompile, observe the
     SnarkAccountUpdate land on OL with the withdrawal intent.
  4. OL to user wallet: simulate the operator picking up the withdrawal
     assignment, build a withdrawal-fulfillment tx via strata-test-cli,
     broadcast it, and assert the recipient bitcoin address received the
     funds.

Bridge denomination and operator fee are read from `asm-params.json`.
Operator keys come from the strata datadir's `bridge-operator_keys` file,
which the strata factory has already wired into the bridge subprotocol's
genesis state.
"""

import logging
import re
import subprocess
import time
from decimal import Decimal
from pathlib import Path
from typing import cast

import flexitest
from eth_account import Account

from common.base_test import BaseTest
from common.bridge import submit_real_bridge_deposit
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.precompile import PRECOMPILE_BRIDGEOUT_ADDRESS, wait_for_receipt
from common.rpc import RpcError
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinProps, BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

SATS_TO_WEI = 10**10

# BOSD descriptor type tags (see strata-common/bitcoin-bosd).
BOSD_P2WPKH_TAG = "03"

# Bridgeout calldata: [4 bytes: selected_operator (big-endian u32)][BOSD bytes].
# 0xFFFFFFFF = u32::MAX = "no specific operator, bridge picks".
NO_OPERATOR_SELECTION_HEX = "ffffffff"

OPERATOR_KEYS_FILENAME = "bridge-operator_keys"


def derive_p2wpkh_bosd_hex(btc_rpc) -> tuple[str, str]:
    """Returns `(bitcoin_address, bosd_hex)` for a fresh P2WPKH address.

    BOSD = `[type_tag: 1 byte][20-byte hash160]`. P2WPKH scriptPubKey is
    `0014<hash160>`, so we strip `0014` and prepend the BOSD type tag.
    """
    addr = btc_rpc.proxy.getnewaddress("", "bech32")
    info = btc_rpc.proxy.getaddressinfo(addr)
    spk = info["scriptPubKey"]
    if not spk.startswith("0014") or len(spk) != 4 + 40:
        raise RuntimeError(f"unexpected scriptPubKey for P2WPKH {addr}: {spk}")
    return addr, BOSD_P2WPKH_TAG + spk[4:]


def fund_strata_test_cli_wallet(btc_rpc, fund_btc: float = 12.0) -> str:
    """Fund the strata-test-cli taproot wallet so it can broadcast a WF tx."""
    res = subprocess.run(
        ["strata-test-cli", "get-address", "--index", "0"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if res.returncode != 0:
        raise RuntimeError(
            f"strata-test-cli get-address failed (exit {res.returncode}): {res.stderr.strip()}"
        )
    bdk_addr = res.stdout.strip()
    btc_rpc.proxy.sendtoaddress(bdk_addr, fund_btc)
    miner = btc_rpc.proxy.getnewaddress()
    btc_rpc.proxy.generatetoaddress(1, miner)
    return bdk_addr


def build_and_broadcast_wf(
    btc_rpc,
    recipient_bosd_hex: str,
    amount_sats: int,
    deposit_idx: int,
    btc_rpc_url: str,
    btc_rpc_user: str,
    btc_rpc_password: str,
) -> str:
    """Sign and broadcast a withdrawal-fulfillment tx via strata-test-cli."""
    res = subprocess.run(
        [
            "strata-test-cli",
            "create-withdrawal-fulfillment",
            "--destination",
            recipient_bosd_hex,
            "--amount",
            str(amount_sats),
            "--deposit-idx",
            str(deposit_idx),
            "--btc-url",
            btc_rpc_url,
            "--btc-user",
            btc_rpc_user,
            "--btc-password",
            btc_rpc_password,
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if res.returncode != 0:
        raise RuntimeError(
            f"create-withdrawal-fulfillment failed (exit {res.returncode}): {res.stderr.strip()}"
        )
    wf_hex = res.stdout.strip()
    wf_txid = btc_rpc.proxy.sendrawtransaction(wf_hex)
    logger.info("WF broadcast: txid=%s deposit_idx=%d", wf_txid, deposit_idx)
    return wf_txid


def wait_for_wf_broadcast(
    btc_rpc,
    bitcoin_props: BitcoinProps,
    miner_addr: str,
    recipient_bosd_hex: str,
    amount_sats: int,
    deposit_idx: int,
    timeout: int = 600,
    blocks_per_step: int = 8,
    poll: float = 1.0,
) -> str:
    """Mine until the bridge assignment is usable, then broadcast the WF tx."""
    deadline = time.time() + timeout
    last_error = "withdrawal fulfillment was not attempted"
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(blocks_per_step, miner_addr)
        try:
            return build_and_broadcast_wf(
                btc_rpc,
                recipient_bosd_hex=recipient_bosd_hex,
                amount_sats=amount_sats,
                deposit_idx=deposit_idx,
                btc_rpc_url=bitcoin_props["rpc_url"],
                btc_rpc_user=bitcoin_props["rpc_user"],
                btc_rpc_password=bitcoin_props["rpc_password"],
            )
        except RuntimeError as e:
            last_error = str(e)
            logger.debug("WF not ready yet: %s", last_error)
        time.sleep(poll)
    raise AssertionError(
        f"could not create and broadcast WF tx for deposit_idx={deposit_idx} "
        f"within {timeout}s; last error: {last_error}"
    )


def get_ol_balance(rpc, account_id_hex: str) -> int:
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    return summaries[0]["balance"] if summaries else 0


def _read_asm_params_int(strata_service: StrataService, key: str) -> int:
    """Recursively scan `asm-params.json` for the first integer field named `key`.

    Tolerates list-of-dict or flat-dict shapes so the schema can move
    Bridge fields under `subprotocols[N].Bridge.*` without breaking us.
    """
    import json

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
    as `read_operator_fee`."""
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


def wait_for_account_update_seq(
    rpc,
    account_id_hex: str,
    min_next_seq_no: int,
    start_epoch: int,
    btc_rpc,
    miner_addr: str,
    timeout: int = 600,
    blocks_per_step: int = 4,
    poll: float = 1.0,
) -> int:
    """Wait until OL terminal epoch summaries include the submitted update."""
    deadline = time.time() + timeout
    last_terminal_epoch = start_epoch
    last_seen_seq_no = -1
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(blocks_per_step, miner_addr)
        time.sleep(poll)
        status = rpc.strata_getChainStatus()
        latest = status["latest"]
        last_terminal_epoch = int(latest["epoch"])
        for epoch in range(start_epoch, last_terminal_epoch + 1):
            try:
                summary = rpc.strata_getAccountEpochSummary(account_id_hex, epoch)
            except RpcError:
                continue
            updates = (summary.get("update_inputs") or []) if summary else []
            for update in updates:
                seq_no = int(update.get("seq_no", -1))
                last_seen_seq_no = max(last_seen_seq_no, seq_no)
                if seq_no >= min_next_seq_no:
                    return epoch
    raise AssertionError(
        f"account update seq_no >= {min_next_seq_no} not found from epoch {start_epoch}; "
        f"last_terminal_epoch={last_terminal_epoch}, last_seen_seq_no={last_seen_seq_no}"
    )


def submit_deposits_and_assert_ol_credit(
    btc_rpc,
    strata_rpc,
    strata_seq: StrataService,
    operator_xprivs: list[str],
    recipient_addr_hex: str,
    miner_addr: str,
    slots_per_epoch: int,
    bridge_denom_sats: int,
    deposit_count: int,
) -> int:
    """Submit `deposit_count` deposits and wait for OL snark account credit."""
    expected_ol_balance_sats = deposit_count * bridge_denom_sats

    for dt_idx in range(deposit_count):
        drt_txid, dt_txid, _ = submit_real_bridge_deposit(
            btc_rpc,
            operator_xprivs_hex=operator_xprivs,
            alpen_address_hex=recipient_addr_hex,
            dt_index=dt_idx,
        )
        logger.info(
            "deposit %d/%d submitted drt=%s dt=%s", dt_idx + 1, deposit_count, drt_txid, dt_txid
        )
        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        strata_seq.wait_for_additional_blocks(2 * slots_per_epoch, strata_rpc, timeout_per_block=15)

    wait_until_with_value(
        lambda: get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID),
        lambda b: b == expected_ol_balance_sats,
        error_with=f"OL not credited with {expected_ol_balance_sats} sats",
        timeout=120,
    )
    logger.info("OL snark account balance = %d sats", expected_ol_balance_sats)
    return expected_ol_balance_sats


def assert_inbox_messages_in_summaries(strata_rpc) -> None:
    """Guards STR-3025: `strata_getBlocksSummaries` must surface inbox messages."""
    chain_status = strata_rpc.strata_getChainStatus()
    tip_slot = chain_status["tip"]["slot"]
    scan_start = max(0, tip_slot - 32)
    summaries = strata_rpc.strata_getBlocksSummaries(ALPEN_ACCOUNT_ID, scan_start, tip_slot)
    total_new_msgs = sum(len(s.get("new_inbox_messages") or []) for s in summaries)
    if total_new_msgs < 1:
        raise AssertionError(
            f"0 inbox messages across slots [{scan_start}, {tip_slot}]; STR-3025 regressed"
        )
    logger.info("getBlocksSummaries surfaced %d inbox message(s)", total_new_msgs)


def wait_for_ee_balance_exact(
    alpen_rpc,
    btc_rpc,
    miner_addr: str,
    deposit_recipient_addr: str,
    expected_wei: int,
    timeout: float = 600,
) -> None:
    """Wait for the EVM balance to reach exactly `expected_wei` (over-credit fails)."""
    deadline = time.time() + timeout
    last_balance = 0
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        time.sleep(1)
        last_balance = int(alpen_rpc.eth_getBalance(deposit_recipient_addr, "latest"), 16)
        if last_balance >= expected_wei:
            break
    else:
        # Hint: strata confirmed/finalized epoch stalls under SAU stream is
        # the usual culprit when the OL -> EE tracker stops advancing here.
        raise AssertionError(
            f"OL -> EE did not credit {deposit_recipient_addr}: "
            f"got {last_balance} wei, expected >= {expected_wei} after {timeout}s"
        )
    if last_balance != expected_wei:
        raise AssertionError(
            f"EVM balance overshot: got {last_balance} wei, expected exactly {expected_wei}"
        )
    logger.info("EVM balance = %d wei (exact)", last_balance)


def submit_bridgeout_and_wait_for_sau(
    alpen_rpc,
    btc_rpc,
    ee_log: Path,
    deposit_recipient_addr: str,
    deposit_recipient_privkey_hex: str,
    recipient_bosd_hex: str,
    withdraw_sats: int,
    miner_addr: str,
    ee_output_log_offset: int,
) -> int:
    """Call the bridgeout precompile and wait for the alpen-client SAU. Returns seq_no."""
    chain_id = int(alpen_rpc.eth_chainId(), 16)
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    nonce = int(alpen_rpc.eth_getTransactionCount(deposit_recipient_addr, "latest"), 16)

    withdraw_tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": 200_000,
        "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
        "value": withdraw_sats * SATS_TO_WEI,
        "data": bytes.fromhex(NO_OPERATOR_SELECTION_HEX + recipient_bosd_hex),
        "chainId": chain_id,
    }
    signed = Account.sign_transaction(withdraw_tx, deposit_recipient_privkey_hex)
    w_hash = alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
    w_receipt = wait_for_receipt(alpen_rpc, w_hash, timeout=30)
    if w_receipt["status"] not in (1, "0x1"):
        raise AssertionError(f"bridgeout call reverted: {w_receipt}")
    if not w_receipt["logs"]:
        raise AssertionError("bridgeout did not emit WithdrawalIntentEvent")

    submitted_seq_no = wait_for_output_snark_update(
        ee_log,
        btc_rpc,
        miner_addr,
        after_offset=ee_output_log_offset,
        timeout=600,
    )
    logger.info("alpen-client submitted SAU seq_no=%d", submitted_seq_no)
    return submitted_seq_no


def wait_for_wf_settlement(
    btc_rpc,
    bitcoin_props: BitcoinProps,
    miner_addr: str,
    withdraw_sats: int,
    operator_fee_sats: int,
    recipient_btc_addr: str,
    recipient_bosd_hex: str,
    recipient_btc_balance_before,
) -> None:
    """Broadcast the WF tx once the bridge assignment exists and verify credit."""
    fund_strata_test_cli_wallet(btc_rpc, fund_btc=12.0)
    wf_txid = wait_for_wf_broadcast(
        btc_rpc,
        bitcoin_props,
        miner_addr,
        recipient_bosd_hex=recipient_bosd_hex,
        amount_sats=withdraw_sats - operator_fee_sats,
        deposit_idx=0,
    )
    btc_rpc.proxy.generatetoaddress(1, miner_addr)
    wait_until_with_value(
        lambda: btc_rpc.proxy.getrawtransaction(wf_txid, 1),
        lambda tx: tx.get("confirmations", 0) >= 1,
        error_with=f"WF tx {wf_txid} did not confirm on bitcoin",
        timeout=60,
    )

    recipient_btc_balance_after = btc_rpc.proxy.getreceivedbyaddress(recipient_btc_addr, 1)
    received_delta_sats = int(
        (recipient_btc_balance_after - recipient_btc_balance_before) * Decimal(100_000_000)
    )
    expected_min_sats = withdraw_sats - operator_fee_sats
    if received_delta_sats < expected_min_sats:
        raise AssertionError(
            f"recipient {recipient_btc_addr} received {received_delta_sats} sats, "
            f"expected >= {expected_min_sats}"
        )
    logger.info("WF %s settled %d sats to %s", wf_txid, received_delta_sats, recipient_btc_addr)


@flexitest.register
class TestRealBridgeDepositWithdraw(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        # `el_ol_bridge` mirrors `el_ol` but runs the OL with a 500ms block
        # time so the deposit -> bridgeout -> checkpoint -> WF cycle fits
        # in a reasonable test runtime.
        ctx.set_env("el_ol_bridge")

    def main(self, ctx):
        del ctx
        strata_seq = cast(StrataService, self.get_service(ServiceType.Strata))
        alpen_seq = cast(AlpenClientService, self.get_service(ServiceType.AlpenSequencer))
        bitcoin = cast(BitcoinService, self.get_service(ServiceType.Bitcoin))

        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=30)
        alpen_rpc = alpen_seq.create_rpc()
        btc_rpc = bitcoin.create_rpc()

        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,  # type: ignore[arg-type]
            rpc=strata_rpc,
            timeout=30,
        )

        operator_xprivs = read_operator_xprivs(strata_seq)
        operator_fee_sats = read_operator_fee(strata_seq)
        bridge_denom_sats = read_bridge_denomination(strata_seq)
        logger.info(
            "asm-params: bridge_denom=%d sats operator_fee=%d sats; %d operator key(s)",
            bridge_denom_sats,
            operator_fee_sats,
            len(operator_xprivs),
        )

        # Fresh recipient with no chainspec prefund so the EE balance check
        # can only pass via the OL -> EE deposit path.
        deposit_recipient = Account.create()
        deposit_recipient_addr = deposit_recipient.address
        recipient_addr_hex = deposit_recipient_addr[2:].lower()

        assert get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID) == 0, "expected 0 OL starting balance"
        ee_balance_before = int(alpen_rpc.eth_getBalance(deposit_recipient_addr, "latest"), 16)
        assert ee_balance_before == 0, (
            f"fresh recipient has non-zero EE balance {ee_balance_before}"
        )

        # Two deposits: the second's worth funds gas for the bridgeout call
        # without needing an external top-up.
        deposit_count = 2
        miner_addr = btc_rpc.proxy.getnewaddress()
        slots_per_epoch = strata_seq.props.get("slots_per_epoch", 5)

        expected_ol_balance_sats = submit_deposits_and_assert_ol_credit(
            btc_rpc,
            strata_rpc,
            strata_seq,
            operator_xprivs=operator_xprivs,
            recipient_addr_hex=recipient_addr_hex,
            miner_addr=miner_addr,
            slots_per_epoch=slots_per_epoch,
            bridge_denom_sats=bridge_denom_sats,
            deposit_count=deposit_count,
        )

        assert_inbox_messages_in_summaries(strata_rpc)

        wait_for_ee_balance_exact(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            deposit_recipient_addr=deposit_recipient_addr,
            expected_wei=expected_ol_balance_sats * SATS_TO_WEI,
        )

        recipient_btc_addr, recipient_bosd_hex = derive_p2wpkh_bosd_hex(btc_rpc)
        recipient_btc_balance_before = btc_rpc.proxy.getreceivedbyaddress(recipient_btc_addr, 1)

        ee_log = ee_log_path(alpen_seq)
        ee_output_log_offset = ee_log.stat().st_size if ee_log.exists() else 0
        start_terminal_epoch = int(strata_rpc.strata_getChainStatus()["latest"]["epoch"])

        submitted_seq_no = submit_bridgeout_and_wait_for_sau(
            alpen_rpc,
            btc_rpc,
            ee_log,
            deposit_recipient_addr=deposit_recipient_addr,
            deposit_recipient_privkey_hex=deposit_recipient.key.hex(),
            recipient_bosd_hex=recipient_bosd_hex,
            withdraw_sats=bridge_denom_sats,
            miner_addr=miner_addr,
            ee_output_log_offset=ee_output_log_offset,
        )

        saw_update_at_epoch = wait_for_account_update_seq(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            min_next_seq_no=submitted_seq_no,
            start_epoch=start_terminal_epoch,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
            timeout=600,
        )
        logger.info(
            "withdrawal-output SnarkAccountUpdate landed at OL epoch %d",
            saw_update_at_epoch,
        )

        wait_for_wf_settlement(
            btc_rpc,
            bitcoin.props,
            miner_addr=miner_addr,
            withdraw_sats=bridge_denom_sats,
            operator_fee_sats=operator_fee_sats,
            recipient_btc_addr=recipient_btc_addr,
            recipient_bosd_hex=recipient_bosd_hex,
            recipient_btc_balance_before=recipient_btc_balance_before,
        )

        # --- Withdrawal cap enforcement ---
        # Use the genesis-prefunded dev account (large balance) to test
        # precompile rejection without worrying about insufficient funds.
        from common.config.constants import DEV_PRIVATE_KEY
        from common.evm import DEV_ACCOUNT_ADDRESS

        chain_id = int(alpen_rpc.eth_chainId(), 16)
        gas_price = int(alpen_rpc.eth_gasPrice(), 16)
        dev_nonce = int(alpen_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        calldata_hex = NO_OPERATOR_SELECTION_HEX + recipient_bosd_hex

        # Over-cap: 11 × denomination exceeds the 10 BTC default cap.
        over_cap_sats = 11 * bridge_denom_sats
        logger.info("bridgeout cap test: %d sats (over cap, expect revert)", over_cap_sats)
        overcap_tx = {
            "nonce": dev_nonce,
            "gasPrice": gas_price,
            "gas": 200_000,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": over_cap_sats * SATS_TO_WEI,
            "data": bytes.fromhex(calldata_hex),
            "chainId": chain_id,
        }
        signed = Account.sign_transaction(overcap_tx, DEV_PRIVATE_KEY)
        h = alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        r = wait_for_receipt(alpen_rpc, h, timeout=30)
        assert r["status"] in (0, "0x0"), f"over-cap bridgeout should revert: {r['status']}"
        logger.info("  over-cap bridgeout reverted as expected")
        dev_nonce += 1

        # Non-multiple: half a denomination is not a valid multiple.
        half_denom_sats = bridge_denom_sats // 2
        logger.info("bridgeout cap test: %d sats (non-multiple, expect revert)", half_denom_sats)
        nonmult_tx = {
            "nonce": dev_nonce,
            "gasPrice": gas_price,
            "gas": 200_000,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": half_denom_sats * SATS_TO_WEI,
            "data": bytes.fromhex(calldata_hex),
            "chainId": chain_id,
        }
        signed = Account.sign_transaction(nonmult_tx, DEV_PRIVATE_KEY)
        h = alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        r = wait_for_receipt(alpen_rpc, h, timeout=30)
        assert r["status"] in (0, "0x0"), f"non-multiple bridgeout should revert: {r['status']}"
        logger.info("  non-multiple bridgeout reverted as expected")

        return True

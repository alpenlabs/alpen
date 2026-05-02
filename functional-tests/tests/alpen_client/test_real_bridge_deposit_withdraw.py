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

Bridge denomination is fixed at 10 BTC (`BRIDGE_OUT_AMOUNT`). Operator keys
come from the strata datadir's `bridge-operator_keys` file, which the
strata factory has already wired into the bridge subprotocol's genesis
state.
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
from common.services.bitcoin import BitcoinService
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
    """Generate a fresh P2WPKH address from bitcoind and convert to BOSD hex.

    Returns `(bitcoin_address, bosd_hex)`. The BOSD format is
    `[type_tag: 1 byte][20-byte hash160]`. The address is bcrt1q-encoded
    P2WPKH; its scriptPubKey is `0014<hash160>`, so we strip the `0014`
    prefix to recover the hash and prepend the BOSD type tag.
    """
    addr = btc_rpc.proxy.getnewaddress("", "bech32")
    info = btc_rpc.proxy.getaddressinfo(addr)
    spk = info["scriptPubKey"]
    if not spk.startswith("0014") or len(spk) != 4 + 40:
        raise RuntimeError(f"unexpected scriptPubKey for P2WPKH {addr}: {spk}")
    hash20_hex = spk[4:]
    return addr, BOSD_P2WPKH_TAG + hash20_hex


def fund_strata_test_cli_wallet(btc_rpc, fund_btc: float = 12.0) -> str:
    """Send BTC to the strata-test-cli internal taproot wallet so it can
    fund a withdrawal-fulfillment tx. Returns the funded address.

    The strata-test-cli wallet is hardcoded to a single XPRIV in
    `strata-test-cli/src/constants.rs`; we get its index-0 address via the
    `get-address` subcommand and send funds to it.
    """
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


def get_ol_balance(rpc, account_id_hex: str) -> int:
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    return summaries[0]["balance"] if summaries else 0


def _read_asm_params_int(strata_service: StrataService, key: str) -> int:
    """Walks `asm-params.json` for the first integer field named `key`.

    The schema nests Bridge fields under `subprotocols[N].Bridge.*`; the
    walk tolerates either list-of-dict or flat-dict shapes so a future
    rearrangement that does not move the field still works.
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
    """Bridge `operator_fee` (sats) from asm-params.

    The bridge assignment stores `net_amount = withdrawal_amount - operator_fee`
    and `validate_withdrawal_fulfillment_info` requires the WF user output
    value to equal that exactly. Hardcoding silently breaks when datatool
    defaults or a test override change.
    """
    return _read_asm_params_int(strata_service, "operator_fee")


def read_bridge_denomination(strata_service: StrataService) -> int:
    """Bridge `denomination` (sats) from asm-params.

    The deposit + withdrawal flow is a fixed-denomination protocol; both
    DT output 1 and WF user output sums are validated against this value.
    Reading from asm-params keeps the test aligned with whatever datatool
    wrote (default or override).
    """
    return _read_asm_params_int(strata_service, "denomination")


def read_operator_xprivs(strata_service: StrataService) -> list[str]:
    """Read the operator BIP32 xprivs from the strata datadir.

    The strata factory generates this file before booting the node and uses
    its contents to populate the bridge subprotocol's genesis operator set.
    Reading the same file means the keys we sign DTs with always match the
    on-chain bridge state.

    The file is one xpriv per line (`tprv...` / `xprv...`), with optional
    trailing whitespace and blank lines that are ignored. We validate that
    every non-blank line at least looks like a BIP32 base58 xpriv prefix so a
    binary or otherwise malformed file fails loudly here instead of much
    later in the DT signing path.
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


def strata_log_path(strata_service: StrataService) -> Path:
    """Path to the current strata runtime log produced by tracing."""
    datadir = Path(strata_service.props["datadir"])
    mode = strata_service.props.get("mode", "sequencer")
    logs = sorted(
        datadir.glob(f"strata-{mode}.*"),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    return logs[0] if logs else datadir / "service.log"


def ee_log_path(alpen_service: AlpenClientService) -> Path:
    """Path to the alpen-client service log produced by the test harness."""
    return Path(alpen_service.props["datadir"]) / "service.log"


# Matches ANSI escape sequences (color codes wrapping log text). The strata
# `tracing` setup writes coloured output even to file logs, so we strip them
# before regex matching.
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def _strip_ansi(text: str) -> str:
    return _ANSI_RE.sub("", text)


def wait_for_log_pattern_with_mining(
    log_path: Path,
    pattern: re.Pattern,
    btc_rpc,
    miner_addr: str,
    after_offset: int,
    timeout: int = 600,
    blocks_per_step: int = 8,
    poll: float = 0.5,
) -> str:
    """Tail a service log for the first occurrence of `pattern` past
    `after_offset` bytes, mining bitcoin blocks each step so the EE/OL/ASM
    pipelines keep advancing. Returns the matched line; raises on timeout.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        if log_path.exists():
            with open(log_path, "rb") as f:
                f.seek(after_offset)
                tail = _strip_ansi(f.read().decode(errors="replace"))
            m = pattern.search(tail)
            if m:
                return m.group(0)
        btc_rpc.proxy.generatetoaddress(blocks_per_step, miner_addr)
        time.sleep(poll)
    raise AssertionError(f"pattern {pattern.pattern!r} not found in {log_path} within {timeout}s")


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
        latest_terminal = status["latest_terminal"]
        last_terminal_epoch = int(latest_terminal["epoch"])
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


def wait_for_checkpoint_at_epoch(
    log_path: Path,
    target_epoch: int,
    btc_rpc,
    miner_addr: str,
    after_offset: int = 0,
    timeout: int = 300,
    blocks_per_step: int = 8,
    poll: float = 0.5,
) -> int:
    """Mine bitcoin blocks until the ASM checkpoint subprotocol logs a
    `checkpoint validated successfully epoch=N` line with `N >= target_epoch`.

    The bridge subprotocol creates a withdrawal assignment in the same ASM
    transition that validates the checkpoint carrying the bridgeout intent.
    Returning here means the assignment has been created and a WF tx will
    pass `validate_withdrawal_fulfillment_info` instead of being rejected
    with `NoAssignmentFound`.

    Returns the matching epoch number.
    """
    pattern = re.compile(r"checkpoint validated successfully\s+epoch=(\d+)")
    deadline = time.time() + timeout
    while time.time() < deadline:
        if log_path.exists():
            with open(log_path, "rb") as f:
                f.seek(after_offset)
                tail = _strip_ansi(f.read().decode(errors="replace"))
            for match in pattern.finditer(tail):
                epoch = int(match.group(1))
                if epoch >= target_epoch:
                    return epoch
        btc_rpc.proxy.generatetoaddress(blocks_per_step, miner_addr)
        time.sleep(poll)
    raise AssertionError(
        f"no `checkpoint validated successfully epoch>={target_epoch}` "
        f"in {log_path} within {timeout}s"
    )


@flexitest.register
class TestRealBridgeDepositWithdraw(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        # `el_ol_bridge` mirrors `el_ol` but runs the OL with a 500ms block
        # time so the deposit -> bridgeout -> checkpoint -> WF cycle fits in
        # a reasonable test runtime.
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
        logger.info("loaded %d operator xpriv(s)", len(operator_xprivs))

        # Read the actual operator_fee from asm-params.json instead of
        # hardcoding it. The factory writes asm-params under the strata
        # datadir; this keeps the test aligned with whatever value
        # datatool wrote (default or override) and fails loudly if the
        # field disappears or moves rather than silently sending the
        # wrong WF amount.
        operator_fee_sats = read_operator_fee(strata_seq)
        bridge_denom_sats = read_bridge_denomination(strata_seq)
        # The deposit -> bridgeout cycle is a fixed-denomination protocol; the
        # full denom is what gets deposited, withdrawn, and split into
        # `(net_amount, operator_fee)` on the WF side.
        withdraw_sats = bridge_denom_sats
        logger.info(
            "asm-params: bridge_denomination=%d sats operator_fee=%d sats",
            bridge_denom_sats,
            operator_fee_sats,
        )

        # Use a freshly-derived recipient with no chainspec prefund so the
        # EE balance assertion can only pass via the OL -> EE deposit path.
        deposit_recipient = Account.create()
        deposit_recipient_addr = deposit_recipient.address
        deposit_recipient_privkey_hex = deposit_recipient.key.hex()
        recipient_addr_hex = deposit_recipient_addr[2:].lower()
        logger.info("fresh deposit recipient: %s (no prefund)", deposit_recipient_addr)

        # ----- bullet 1: real-bridge deposit credits OL snark account -----
        # Two deposits land 20 BTC on the recipient. Bullet 3's bridgeout sends
        # 10 BTC; the second deposit's worth funds gas without external top-up.
        initial_ol_balance = get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID)
        assert initial_ol_balance == 0, f"expected 0 starting balance, got {initial_ol_balance}"

        # The fresh recipient starts at zero, so the EE balance check depends
        # on the deposit crossing.
        ee_balance_before = int(alpen_rpc.eth_getBalance(deposit_recipient_addr, "latest"), 16)
        if ee_balance_before != 0:
            raise AssertionError(
                f"fresh recipient {deposit_recipient_addr} unexpectedly has "
                f"non-zero balance {ee_balance_before}; address collision?"
            )

        deposit_count = 2
        expected_ol_balance_sats = deposit_count * bridge_denom_sats
        miner_addr = btc_rpc.proxy.getnewaddress()
        slots_per_epoch = strata_seq.props.get("slots_per_epoch", 5)

        for dt_idx in range(deposit_count):
            drt_txid, dt_txid, _drt = submit_real_bridge_deposit(
                btc_rpc,
                operator_xprivs_hex=operator_xprivs,
                alpen_address_hex=recipient_addr_hex,
                dt_index=dt_idx,
            )
            logger.info(
                "real-bridge deposit %d/%d submitted drt=%s dt=%s",
                dt_idx + 1,
                deposit_count,
                drt_txid,
                dt_txid,
            )

            # Mine to confirm the DT and let ASM bridge process it. The OL
            # then consumes the manifest and credits the snark account.
            btc_rpc.proxy.generatetoaddress(8, miner_addr)
            strata_seq.wait_for_additional_blocks(
                2 * slots_per_epoch, strata_rpc, timeout_per_block=15
            )

        wait_until_with_value(
            lambda: get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID),
            lambda b: b == expected_ol_balance_sats,
            error_with=(
                f"OL snark account not credited with {expected_ol_balance_sats} sats "
                f"({deposit_count} x {bridge_denom_sats})"
            ),
            timeout=120,
        )
        logger.info(
            "[1] OL snark account balance = %d sats (%d deposits)",
            expected_ol_balance_sats,
            deposit_count,
        )

        # ----- bullet 1b: RPC surfaces the deposit inbox message -----
        # `getBlocksSummaries` must report the inbox message consumed by
        # alpen-client during the OL -> EE crossing.
        chain_status = strata_rpc.strata_getChainStatus()
        tip_slot = chain_status["tip"]["slot"]
        scan_start = max(0, tip_slot - 32)
        summaries = strata_rpc.strata_getBlocksSummaries(ALPEN_ACCOUNT_ID, scan_start, tip_slot)
        total_new_msgs = sum(len(s.get("new_inbox_messages") or []) for s in summaries)
        if total_new_msgs < 1:
            raise AssertionError(
                f"strata_getBlocksSummaries surfaced 0 inbox messages across "
                f"slots [{scan_start}, {tip_slot}] - either the read-side "
                f"(node.rs) or the write-side (chain-worker-new/context.rs) "
                f"of the STR-3025 fix has regressed."
            )
        logger.info(
            "[1b] strata_getBlocksSummaries surfaced %d inbox message(s) across slots [%d, %d]",
            total_new_msgs,
            scan_start,
            tip_slot,
        )

        # ----- bullet 2: OL to EE - EVM balance equals 2 x bridge denom -----
        # The fresh recipient starts with no prefund. Both deposits must
        # cross OL -> EE, so the balance lands at exactly two denominations.
        expected_wei = expected_ol_balance_sats * SATS_TO_WEI
        deadline = time.time() + 600
        last_balance = 0
        while time.time() < deadline:
            btc_rpc.proxy.generatetoaddress(8, miner_addr)
            time.sleep(1)
            last_balance = int(alpen_rpc.eth_getBalance(deposit_recipient_addr, "latest"), 16)
            if last_balance >= expected_wei:
                break
        else:
            raise AssertionError(
                f"OL -> EE crossing did not credit {deposit_recipient_addr}: "
                f"got {last_balance} wei, expected >= {expected_wei} after 600s. "
                f"Likely culprit: strata `confirmed_epoch`/`finalized_epoch` "
                f"stalls under SAU stream so the alpen-client OL chain tracker "
                f"never advances past the deposit's epoch and inbox messages "
                f"are never consumed."
            )
        if last_balance != expected_wei:
            raise AssertionError(
                f"EVM balance overshot the deposited amount: got {last_balance} wei, "
                f"expected exactly {expected_wei} wei from {deposit_count} deposits"
            )
        logger.info(
            "[2] EVM balance for fresh recipient %s = %d wei (exact, %d deposits)",
            deposit_recipient_addr,
            last_balance,
            deposit_count,
        )

        # ----- bullet 3: EE to OL - bridgeout precompile -----
        # Generate a real recipient bitcoin address now so bullet 4 can verify
        # delivery to it. The BOSD form is what the bridge subprotocol records
        # as the withdrawal target.
        recipient_btc_addr, recipient_bosd_hex = derive_p2wpkh_bosd_hex(btc_rpc)
        logger.info(
            "withdrawal recipient: btc_addr=%s bosd=%s",
            recipient_btc_addr,
            recipient_bosd_hex,
        )
        recipient_btc_balance_before = btc_rpc.proxy.getreceivedbyaddress(recipient_btc_addr, 1)

        strata_log = strata_log_path(strata_seq)
        ee_log = ee_log_path(alpen_seq)
        strata_checkpoint_log_offset = strata_log.stat().st_size if strata_log.exists() else 0
        ee_output_log_offset = ee_log.stat().st_size if ee_log.exists() else 0
        start_terminal_epoch = int(strata_rpc.strata_getChainStatus()["latest_terminal"]["epoch"])

        # Bridgeout from the fresh recipient, using bridged-in BTC for both
        # the withdraw `value` and gas. The two deposits leave the recipient
        # with 2x the bridge denomination; the second deposit's worth covers
        # gas without any external top-up.
        chain_id = int(alpen_rpc.eth_chainId(), 16)
        gas_price = int(alpen_rpc.eth_gasPrice(), 16)
        nonce = int(alpen_rpc.eth_getTransactionCount(deposit_recipient_addr, "latest"), 16)
        withdraw_wei = withdraw_sats * SATS_TO_WEI

        withdraw_tx = {
            "nonce": nonce,
            "gasPrice": gas_price,
            "gas": 200_000,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": withdraw_wei,
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
        logger.info("[3] bridgeout precompile emitted WithdrawalIntentEvent")

        submitted_seq_no = wait_for_output_snark_update(
            ee_log,
            btc_rpc,
            miner_addr,
            after_offset=ee_output_log_offset,
            timeout=600,
        )
        logger.info(
            "[3b] alpen-client submitted withdrawal-output SnarkAccountUpdate seq_no=%d",
            submitted_seq_no,
        )

        saw_update_at_epoch = wait_for_account_update_seq(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            min_next_seq_no=submitted_seq_no + 1,
            start_epoch=start_terminal_epoch,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
            timeout=600,
        )
        logger.info(
            "[3c] withdrawal-output SnarkAccountUpdate landed at OL epoch %d",
            saw_update_at_epoch,
        )

        # ----- bullet 4: OL to user wallet - withdrawal-fulfillment -----
        validated_epoch = wait_for_checkpoint_at_epoch(
            strata_log,
            saw_update_at_epoch,
            btc_rpc,
            miner_addr,
            after_offset=strata_checkpoint_log_offset,
            timeout=600,
        )
        logger.info("[4a] ASM checkpoint validated OL epoch %d", validated_epoch)

        fund_strata_test_cli_wallet(btc_rpc, fund_btc=12.0)

        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        strata_seq.wait_for_additional_blocks(2 * slots_per_epoch, strata_rpc, timeout_per_block=15)

        fulfillment_log_offset = strata_log.stat().st_size if strata_log.exists() else 0
        wf_txid = build_and_broadcast_wf(
            btc_rpc,
            recipient_bosd_hex=recipient_bosd_hex,
            amount_sats=withdraw_sats - operator_fee_sats,
            deposit_idx=0,
            btc_rpc_url=bitcoin.props["rpc_url"],
            btc_rpc_user=bitcoin.props["rpc_user"],
            btc_rpc_password=bitcoin.props["rpc_password"],
        )
        logger.info("[4b] WF broadcast txid=%s recipient=%s", wf_txid, recipient_btc_addr)

        wait_for_log_pattern_with_mining(
            strata_log,
            re.compile(
                r"(Fulfilled withdrawal assignment\b.*deposit_idx=0|"
                r"deposit_idx=0.*Fulfilled withdrawal assignment\b)"
            ),
            btc_rpc,
            miner_addr,
            after_offset=fulfillment_log_offset,
            timeout=600,
        )

        wf_info = btc_rpc.proxy.getrawtransaction(wf_txid, 1)
        if wf_info.get("confirmations", 0) < 1:
            raise AssertionError(f"WF tx {wf_txid} did not confirm on bitcoin")
        logger.info(
            "[4c] WF confirmed in block %s (depth=%d)",
            wf_info.get("blockhash"),
            int(wf_info.get("confirmations", 0)),
        )

        recipient_btc_balance_after = btc_rpc.proxy.getreceivedbyaddress(recipient_btc_addr, 1)
        received_delta_sats = int(
            (recipient_btc_balance_after - recipient_btc_balance_before) * Decimal(100_000_000)
        )
        # Recipient receives the assignment's net amount (denom minus operator fee).
        expected_min_sats = withdraw_sats - operator_fee_sats
        if received_delta_sats < expected_min_sats:
            raise AssertionError(
                f"recipient {recipient_btc_addr} received {received_delta_sats} sats, "
                f"expected at least {expected_min_sats}"
            )
        logger.info(
            "[4] WF %s fulfilled withdrawal assignment and paid %d sats to %s",
            wf_txid,
            received_delta_sats,
            recipient_btc_addr,
        )

        return True

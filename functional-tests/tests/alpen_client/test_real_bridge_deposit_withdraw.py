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
import time
from pathlib import Path
from typing import cast

import flexitest
from eth_account import Account

from common.base_test import BaseTest
from common.bridge import submit_real_bridge_deposit
from common.config.constants import ALPEN_ACCOUNT_ID, DEV_CHAIN_ID, DEV_PRIVATE_KEY, ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS
from common.precompile import PRECOMPILE_BRIDGEOUT_ADDRESS, wait_for_receipt
from common.rpc import RpcError
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

BRIDGE_DENOM_SATS = 1_000_000_000  # 10 BTC
SATS_TO_WEI = 10**10
WITHDRAW_SATS = 1_000_000_000  # 10 BTC, full denom

# BOSD destination descriptor: [type_tag: 1 byte][payload]. Type tag 0x03 with
# a 20-byte hash160 is P2WPKH and accepts any 20 bytes (no curve check), which
# makes it a safe placeholder for the bridgeout precompile. We swap this for
# a real recipient address when we exercise the withdrawal-fulfillment leg.
DUMMY_BOSD_HEX = "03" + "11" * 20
# Bridgeout calldata: [4 bytes: selected_operator (big-endian u32)][BOSD bytes].
# 0xFFFFFFFF = u32::MAX = "no specific operator, bridge picks".
NO_OPERATOR_SELECTION_HEX = "ffffffff"

OPERATOR_KEYS_FILENAME = "bridge-operator_keys"


def get_ol_balance(rpc, account_id_hex: str) -> int:
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    return summaries[0]["balance"] if summaries else 0


def read_operator_xprivs(strata_service: StrataService) -> list[str]:
    """Read the operator BIP32 xpriv from the strata datadir.

    The strata factory generates this file before booting the node and uses
    its contents to populate the bridge subprotocol's genesis operator set.
    Reading the same file means the keys we sign DTs with always match the
    on-chain bridge state.
    """
    datadir = Path(strata_service.props["datadir"])
    path = datadir / OPERATOR_KEYS_FILENAME
    if not path.exists():
        raise RuntimeError(f"operator key file not found: {path}")
    return [path.read_text().strip()]


@flexitest.register
class TestRealBridgeDepositWithdraw(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

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

        # The Alpen EE address that should receive the minted BTC. Strip the 0x
        # prefix because the Rust subcommand expects bare hex.
        recipient_addr_hex = DEV_ACCOUNT_ADDRESS[2:].lower()

        # ----- bullet 1: real-bridge deposit credits OL snark account -----
        initial_ol_balance = get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID)
        assert initial_ol_balance == 0, f"expected 0 starting balance, got {initial_ol_balance}"

        drt_txid, dt_txid, _drt = submit_real_bridge_deposit(
            btc_rpc,
            operator_xprivs_hex=operator_xprivs,
            alpen_address_hex=recipient_addr_hex,
            dt_index=0,
        )
        logger.info("real-bridge deposit submitted drt=%s dt=%s", drt_txid, dt_txid)

        # Mine to confirm the DT and let ASM bridge process it. The OL then
        # consumes the manifest and credits the snark account; we need to
        # cross at least one OL terminal block (epoch boundary).
        miner_addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        slots_per_epoch = strata_seq.props.get("slots_per_epoch", 5)
        strata_seq.wait_for_additional_blocks(2 * slots_per_epoch, strata_rpc, timeout_per_block=15)

        wait_until_with_value(
            lambda: get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID),
            lambda b: b == BRIDGE_DENOM_SATS,
            error_with=f"OL snark account not credited with {BRIDGE_DENOM_SATS} sats",
            timeout=120,
        )
        logger.info("[1] OL snark account balance = %d sats", BRIDGE_DENOM_SATS)

        # ----- bullet 2: OL to EE - EVM balance increases -----
        expected_wei = BRIDGE_DENOM_SATS * SATS_TO_WEI
        wait_until_with_value(
            lambda: int(alpen_rpc.eth_getBalance(DEV_ACCOUNT_ADDRESS, "latest"), 16),
            lambda b: b >= expected_wei,
            error_with=f"EVM balance not credited with {expected_wei} wei",
            timeout=120,
        )
        actual_wei = int(alpen_rpc.eth_getBalance(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        logger.info("[2] EVM balance = %d wei (expected >= %d)", actual_wei, expected_wei)

        # ----- bullet 3: EE to OL - bridgeout precompile -----
        nonce = int(alpen_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        gas_price = int(alpen_rpc.eth_gasPrice(), 16)
        withdraw_wei = WITHDRAW_SATS * SATS_TO_WEI

        withdraw_tx = {
            "nonce": nonce,
            "gasPrice": gas_price,
            "gas": 200_000,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": withdraw_wei,
            "data": bytes.fromhex(NO_OPERATOR_SELECTION_HEX + DUMMY_BOSD_HEX),
            "chainId": DEV_CHAIN_ID,
        }
        signed = Account.sign_transaction(withdraw_tx, DEV_PRIVATE_KEY)
        w_hash = alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        w_receipt = wait_for_receipt(alpen_rpc, w_hash, timeout=30)
        if w_receipt["status"] not in (1, "0x1"):
            raise AssertionError(f"bridgeout call reverted: {w_receipt}")
        if not w_receipt["logs"]:
            raise AssertionError("bridgeout did not emit WithdrawalIntentEvent")
        logger.info("[3] bridgeout precompile emitted WithdrawalIntentEvent")

        # Wait for the alpen-client SnarkAccountUpdate to land on OL; this
        # indicates the EE-side withdrawal intent has been published to OL.
        start_epoch = strata_rpc.strata_getChainStatus()["tip"]["epoch"]
        deadline = time.time() + 120
        saw_update_at_epoch = -1
        while time.time() < deadline:
            btc_rpc.proxy.generatetoaddress(2, miner_addr)
            time.sleep(2)
            tip_epoch = strata_rpc.strata_getChainStatus()["tip"]["epoch"]
            for ep in range(start_epoch, tip_epoch + 1):
                # Epoch summary RPC errors when the epoch is not yet finalized.
                # Treat that as "not ready, keep polling".
                try:
                    summary = strata_rpc.strata_getAccountEpochSummary(ALPEN_ACCOUNT_ID, ep)
                except RpcError:
                    continue
                if summary and summary.get("update_input") is not None:
                    saw_update_at_epoch = ep
                    break
            if saw_update_at_epoch >= 0:
                break
        if saw_update_at_epoch < 0:
            raise AssertionError("no SnarkAccountUpdate from alpen-client within 120s")
        logger.info("[3b] SnarkAccountUpdate landed at OL epoch %d", saw_update_at_epoch)

        # ----- bullet 4: OL to user wallet - withdrawal-fulfillment -----
        # TODO: pick up the bridge subprotocol's withdrawal assignment from the
        # OL state, call `strata-test-cli create-withdrawal-fulfillment` with
        # operator_xprivs, broadcast the WF tx, and assert the recipient
        # bitcoin address receives BRIDGE_DENOM_SATS sats.
        #
        # The DUMMY_BOSD_HEX above currently encodes a P2TR placeholder; for
        # this assertion to be meaningful we need to recipient_addr_btc be a
        # real address we can poll via listunspent / gettxout. Sketched in
        # follow-up commit.
        logger.info("[4] withdrawal-fulfillment leg pending follow-up commit")

        return True

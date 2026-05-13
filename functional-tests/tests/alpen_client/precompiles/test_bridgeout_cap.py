"""Verify the bridgeout precompile enforces the withdrawal cap.

Sends two transactions to the bridgeout precompile:
  1. Over-cap amount (11 BTC) -> expects revert
  2. At-cap amount (10 BTC) -> expects success

Uses the dev account which has a large pre-funded balance.
"""

import logging

import flexitest
from eth_account import Account
from eth_utils import to_checksum_address

from common.base_test import BaseTest
from common.config.constants import DEV_CHAIN_ID, DEV_PRIVATE_KEY, ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS
from common.precompile import PRECOMPILE_BRIDGEOUT_ADDRESS, wait_for_receipt
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

SATS_TO_WEI = 10**10
DENOMINATION_SATS = 100_000_000  # 1 BTC
MAX_WITHDRAWAL_SATS = 1_000_000_000  # 10 BTC

# Bridgeout calldata: [4 bytes: selected_operator (u32 big-endian)][BOSD bytes]
# 0xFFFFFFFF = no operator preference
# 0x03 + 20 bytes = valid P2WPKH BOSD descriptor
NO_OP_HEX = "ffffffff"
VALID_P2WPKH_BOSD_HEX = "03" + "14" * 20


def build_bridgeout_tx(rpc, amount_sats: int, nonce: int) -> dict:
    """Build a bridgeout precompile transaction."""
    gas_price = int(rpc.eth_gasPrice(), 16)
    return {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": 200_000,
        "to": to_checksum_address(PRECOMPILE_BRIDGEOUT_ADDRESS),
        "value": amount_sats * SATS_TO_WEI,
        "data": bytes.fromhex(NO_OP_HEX + VALID_P2WPKH_BOSD_HEX),
        "chainId": DEV_CHAIN_ID,
    }


@flexitest.register
class TestBridgeoutWithdrawalCap(BaseTest):
    """Bridgeout precompile: over-cap reverts, at-cap succeeds."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)

        # --- Test 1: Over-cap (11 BTC) should revert ---
        over_cap_sats = 11 * DENOMINATION_SATS
        logger.info(f"Test 1: bridgeout {over_cap_sats} sats (over cap, expect revert)")

        tx = build_bridgeout_tx(rpc, over_cap_sats, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert receipt["status"] in (0, "0x0"), (
            f"Over-cap bridgeout should revert, got status {receipt['status']}"
        )
        logger.info("  Over-cap bridgeout reverted as expected")
        nonce += 1

        # --- Test 2: Non-multiple of denomination (0.5 BTC) should revert ---
        non_multiple_sats = 50_000_000  # 0.5 BTC
        logger.info(f"Test 2: bridgeout {non_multiple_sats} sats (non-multiple, expect revert)")

        tx = build_bridgeout_tx(rpc, non_multiple_sats, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert receipt["status"] in (0, "0x0"), (
            f"Non-multiple bridgeout should revert, got status {receipt['status']}"
        )
        logger.info("  Non-multiple bridgeout reverted as expected")
        nonce += 1

        # --- Test 3: At-cap (10 BTC) should succeed ---
        at_cap_sats = MAX_WITHDRAWAL_SATS
        logger.info(f"Test 2: bridgeout {at_cap_sats} sats (at cap, expect success)")

        tx = build_bridgeout_tx(rpc, at_cap_sats, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert receipt["status"] in (1, "0x1"), (
            f"At-cap bridgeout should succeed, got status {receipt['status']}"
        )
        assert len(receipt["logs"]) > 0, "At-cap bridgeout should emit WithdrawalIntentEvent"
        logger.info("  At-cap bridgeout succeeded with withdrawal intent log")

        logger.info("Bridgeout cap tests passed")
        return True

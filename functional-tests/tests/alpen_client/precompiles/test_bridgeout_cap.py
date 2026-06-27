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
from common.rpc import RpcError
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

SATS_TO_WEI = 10**10
DENOMINATION_SATS = 100_000_000  # 1 BTC
MAX_WITHDRAWAL_SATS = 1_000_000_000  # 10 BTC

# Gas limit forwarded with each bridgeout tx. A rejected withdrawal should REVERT and
# refund the unspent gas, so a failing tx must consume far less than this. If failures
# instead halted (the old behavior), gasUsed would pin to ~GAS_LIMIT.
GAS_LIMIT = 200_000

# Bridgeout calldata: [4 bytes: selected_operator (u32 big-endian)][BOSD bytes]
# 0xFFFFFFFF = no operator preference
# 0x03 + 20 bytes = valid P2WPKH BOSD descriptor
NO_OP_HEX = "ffffffff"
VALID_P2WPKH_BOSD_HEX = "03" + "14" * 20

# Solidity Error(string) selector, bytes4(keccak256("Error(string)")).
ERROR_STRING_SELECTOR = "08c379a0"


def build_bridgeout_tx(rpc, amount_sats: int, nonce: int) -> dict:
    """Build a bridgeout precompile transaction."""
    gas_price = int(rpc.eth_gasPrice(), 16)
    return {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": GAS_LIMIT,
        "to": to_checksum_address(PRECOMPILE_BRIDGEOUT_ADDRESS),
        "value": amount_sats * SATS_TO_WEI,
        "data": bytes.fromhex(NO_OP_HEX + VALID_P2WPKH_BOSD_HEX),
        "chainId": DEV_CHAIN_ID,
    }


def gas_used(receipt: dict) -> int:
    """Return the receipt's gasUsed as an int (handles hex or int encodings)."""
    used = receipt["gasUsed"]
    return int(used, 16) if isinstance(used, str) else used


def get_revert_reason(rpc, amount_sats: int) -> str | None:
    """Simulate a bridgeout via eth_call and return the revert error text, if any.

    Returns the RPC error string on revert (which carries the decoded Error(string)
    reason and/or the 0x08c379a0 revert payload), or None if the call did not revert.
    """
    try:
        rpc.eth_call(
            {
                "from": DEV_ACCOUNT_ADDRESS,
                "to": to_checksum_address(PRECOMPILE_BRIDGEOUT_ADDRESS),
                "value": hex(amount_sats * SATS_TO_WEI),
                "data": "0x" + NO_OP_HEX + VALID_P2WPKH_BOSD_HEX,
            },
            "latest",
        )
        return None
    except RpcError as e:
        return str(e)


def assert_reverted(rpc, receipt: dict, amount_sats: int, expect_reason: str):
    """Assert a failing bridgeout reverted with gas refunded and the expected reason."""
    status = receipt["status"]
    status = int(status, 16) if isinstance(status, str) else status
    assert status == 0, f"expected failure status, got {status}"

    # A revert refunds unspent gas; a gas-burning halt would consume ~GAS_LIMIT.
    used = gas_used(receipt)
    assert used < GAS_LIMIT // 2, (
        f"expected gas to be refunded on revert, but gasUsed={used} (limit {GAS_LIMIT}) "
        f"— this looks like an all-gas-consuming halt, not a revert"
    )

    # The reason should be recoverable from an eth_call simulation.
    reason = get_revert_reason(rpc, amount_sats)
    assert reason is not None, "eth_call did not revert"
    reason_l = reason.lower()
    assert expect_reason.lower() in reason_l or ERROR_STRING_SELECTOR in reason_l, (
        f"revert reason {reason!r} missing expected text {expect_reason!r} / selector"
    )


@flexitest.register
class TestBridgeoutWithdrawalCap(BaseTest):
    """Bridgeout precompile: over-cap reverts, at-cap succeeds."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)

        # --- Test 1: Over-cap (11 BTC) should revert with gas refunded ---
        over_cap_sats = 11 * DENOMINATION_SATS
        logger.info(f"Test 1: bridgeout {over_cap_sats} sats (over cap, expect revert)")

        tx = build_bridgeout_tx(rpc, over_cap_sats, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert_reverted(rpc, receipt, over_cap_sats, expect_reason="exact multiple")
        logger.info(
            f"  Over-cap bridgeout reverted as expected, gasUsed={gas_used(receipt)} (refunded)"
        )
        nonce += 1

        # --- Test 2: Non-multiple of denomination (0.5 BTC) should revert with gas refunded ---
        non_multiple_sats = 50_000_000  # 0.5 BTC
        logger.info(f"Test 2: bridgeout {non_multiple_sats} sats (non-multiple, expect revert)")

        tx = build_bridgeout_tx(rpc, non_multiple_sats, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert_reverted(rpc, receipt, non_multiple_sats, expect_reason="positive exact multiple")
        logger.info(
            f"  Non-multiple bridgeout reverted as expected, gasUsed={gas_used(receipt)} (refunded)"
        )
        nonce += 1

        # --- Test 3: At-cap (10 BTC) should succeed ---
        at_cap_sats = MAX_WITHDRAWAL_SATS
        logger.info(f"Test 3: bridgeout {at_cap_sats} sats (at cap, expect success)")

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

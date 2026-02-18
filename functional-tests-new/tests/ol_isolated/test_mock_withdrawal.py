"""
Test mock deposit and withdrawal via strata-test-cli.

This test verifies the end-to-end flow of depositing funds into a snark account
via the debug subprotocol and then withdrawing them, all without a real EE.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.test_cli import build_snark_withdrawal, create_mock_deposit

logger = logging.getLogger(__name__)

# Test account reference byte (matches OLIsolatedEnvConfig default)
TEST_ACCOUNT_REF = 0x42
# First user serial: system accounts occupy serials 0-127, so the first
# genesis user account is assigned serial 128.
TEST_ACCOUNT_SERIAL = 128

# Withdrawal denomination: 1 BTC in satoshis
WITHDRAWAL_DENOMINATION_SATS = 100_000_000

# Deposit amount: 20 BTC in satoshis
DEPOSIT_AMOUNT_SATS = 2_000_000_000


def make_test_account_id_hex() -> str:
    """Create the test account ID hex (plain hex, no 0x prefix).

    AccountId uses hex::serde which expects plain hex without 0x prefix.
    """
    return "00" * 31 + f"{TEST_ACCOUNT_REF:02x}"


def get_account_balance(rpc, account_id_hex: str) -> int:
    """Query the account balance at the latest slot.

    Uses getChainStatus to find the latest slot, then getBlocksSummaries
    to get the balance, since getSnarkAccountState does not include balance.
    """
    status = rpc.strata_getChainStatus()
    latest_slot = status["latest"]["slot"]

    summaries = rpc.strata_getBlocksSummaries(account_id_hex, latest_slot, latest_slot)
    if not summaries:
        return 0

    return summaries[0]["balance"]


@flexitest.register
class TestMockWithdrawal(StrataNodeTest):
    """
    Test deposit + withdrawal via strata-test-cli.

    1. Start bitcoind + strata (OL, no EE)
    2. Wait for OL RPC ready
    3. Deposit via create-mock-deposit (debug subprotocol)
    4. Generate Bitcoin blocks to mature the tx
    5. Wait for OL to process the manifest
    6. Assert balance == deposit amount
    7. Build and submit withdrawal via build-snark-withdrawal
    8. Wait for OL blocks
    9. Assert balance == deposit - withdrawal
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("ol_isolated")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=30)

        account_id_hex = make_test_account_id_hex()
        logger.info(f"Test account ID: {account_id_hex}")

        # Get Bitcoin RPC config
        btc_url = bitcoin.props["rpc_url"]
        btc_user = bitcoin.props["rpc_user"]
        btc_password = bitcoin.props["rpc_password"]
        btc_rpc = bitcoin.create_rpc()

        # Step 1: Deposit via debug subprotocol
        logger.info(
            f"Injecting mock deposit: {DEPOSIT_AMOUNT_SATS} sats "
            f"to serial {TEST_ACCOUNT_SERIAL:#x}"
        )
        txid = create_mock_deposit(
            account_serial=TEST_ACCOUNT_SERIAL,
            amount=DEPOSIT_AMOUNT_SATS,
            btc_url=btc_url,
            btc_user=btc_user,
            btc_password=btc_password,
        )
        logger.info(f"Mock deposit broadcast, txid: {txid}")

        # Step 2: Mine Bitcoin blocks to mature the tx and let ASM process it
        logger.info("Mining Bitcoin blocks to mature deposit tx...")
        addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(6, addr)

        # Step 3: Wait for OL to reach a terminal block (epoch boundary).
        # L1 manifests are only processed during terminal blocks. With
        # slots_per_epoch=5 the first post-genesis terminal block is at slot 5.
        # Wait for enough blocks to guarantee we cross at least one terminal
        # boundary after the ASM has processed the new Bitcoin blocks.
        logger.info("Waiting for OL to reach terminal block and process deposit...")
        strata.wait_for_additional_blocks(10, rpc, timeout_per_block=15)

        # Step 4: Query account balance and verify deposit
        balance = get_account_balance(rpc, account_id_hex)
        logger.info(f"Account balance after deposit: {balance} sats")

        if balance != DEPOSIT_AMOUNT_SATS:
            raise AssertionError(
                f"Balance mismatch after deposit: "
                f"expected {DEPOSIT_AMOUNT_SATS}, got {balance}"
            )

        # Step 5: Build withdrawal transaction
        # Get snark account state for withdrawal params
        account_state = rpc.strata_getSnarkAccountState(account_id_hex, "latest")
        if account_state is None:
            raise AssertionError("Account state not found")

        seq_no = account_state["seq_no"]
        next_inbox_idx = account_state["next_inbox_msg_idx"]
        inner_state_hex = account_state["inner_state"]

        withdrawal_dest = b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        dest_hex = withdrawal_dest.hex()

        logger.info(f"Building withdrawal: {WITHDRAWAL_DENOMINATION_SATS} sats")
        tx_json = build_snark_withdrawal(
            target_hex=account_id_hex,
            seq_no=seq_no,
            inner_state_hex=inner_state_hex,
            next_inbox_idx=next_inbox_idx,
            dest_hex=dest_hex,
            amount=WITHDRAWAL_DENOMINATION_SATS,
            fees=0,
        )
        logger.info(f"Built withdrawal tx: {tx_json}")

        # Step 6: Submit withdrawal
        logger.info("Submitting withdrawal transaction...")
        tx_id = rpc.strata_submitTransaction(tx_json)
        logger.info(f"Withdrawal submitted, ID: {tx_id}")

        # Step 7: Wait for OL blocks to process withdrawal
        strata.wait_for_additional_blocks(2, rpc, timeout_per_block=15)

        # Step 8: Verify final balance
        final_balance = get_account_balance(rpc, account_id_hex)
        expected_balance = DEPOSIT_AMOUNT_SATS - WITHDRAWAL_DENOMINATION_SATS

        logger.info(
            f"Balance: {balance} -> {final_balance} (expected: {expected_balance})"
        )

        if final_balance != expected_balance:
            raise AssertionError(
                f"Balance mismatch after withdrawal: "
                f"expected {expected_balance}, got {final_balance}"
            )

        logger.info("Deposit + withdrawal test passed!")
        return True

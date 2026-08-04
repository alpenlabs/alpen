"""
Test snark-account withdrawal via strata-test-cli, without a real EE.

The point of this test is the withdrawal half: building a snark-account
update with `build-snark-withdrawal` and submitting it to the OL. The
deposit is setup, and goes through the real bridge subprotocol — ASM
v0.4.0-rc.1 removed the debug subprotocol that used to accept synthetic
deposit logs, so there is no shortcut any more. Each deposit credits exactly
the configured bridge denomination, so the starting balance is built up from
a whole number of them rather than an arbitrary figure.

Balances are asserted as deltas: this environment is shared with
`test_deposit_unregistered_serial`, which also makes real deposits, so the
account may already hold funds when this test starts.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.bridge import read_bridge_denomination, read_operator_xprivs, submit_real_bridge_deposit
from common.config import ServiceType
from common.test_cli import build_snark_withdrawal
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# Test account reference byte (matches the `ol_isolated` env's genesis account)
TEST_ACCOUNT_REF = 0x42
# First user serial: system accounts occupy serials 0-127, so the first
# genesis user account is assigned serial 128.
TEST_ACCOUNT_SERIAL = 128

# Number of bridge deposits to fund the account with. Each credits exactly the
# bridge denomination, so this must be >= 2 for the withdrawal below to leave a
# non-zero remainder.
DEPOSIT_COUNT = 2

# Bridge deposit indices must be unique across the env.
# `test_deposit_unregistered_serial` shares it and uses the high indices.
FIRST_DT_INDEX = 0

# Destination subject is opaque to balance crediting — only the serial in the
# descriptor selects the account.
DEPOSIT_SUBJECT_HEX = "00" * 20


def make_test_account_id_hex() -> str:
    """Create the test account ID hex (plain hex, no 0x prefix).

    AccountId uses hex::serde which expects plain hex without 0x prefix.
    """
    return "00" * 31 + f"{TEST_ACCOUNT_REF:02x}"


def get_account_balance(rpc, account_id_hex: str) -> int:
    """Query the account balance at the latest slot.

    Uses getChainStatus to find the latest slot, then getBlocksSummaries
    to get the balance, since getSnarkAccountStateByTag does not include balance.
    """
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]

    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    if not summaries:
        return 0

    return summaries[0]["balance"]


@flexitest.register
class TestMockWithdrawal(StrataNodeTest):
    """
    Test bridge deposit + snark-account withdrawal via strata-test-cli.

    1. Start bitcoind + strata (OL, no EE)
    2. Wait for OL RPC ready
    3. Fund the account with DEPOSIT_COUNT real bridge deposits
    4. Assert the balance rose by DEPOSIT_COUNT * denomination
    5. Build and submit a withdrawal of one denomination via build-snark-withdrawal
    6. Assert the balance fell by exactly that withdrawal
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("ol_isolated")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=30)
        submit_rpc = strata.create_submit_rpc()

        account_id_hex = make_test_account_id_hex()
        logger.info(f"Test account ID: {account_id_hex}")

        btc_rpc = bitcoin.create_rpc()
        miner_addr = btc_rpc.proxy.getnewaddress()

        operator_xprivs = read_operator_xprivs(strata)
        denomination = read_bridge_denomination(strata)
        slots_per_epoch = strata.props["slots_per_epoch"]
        logger.info("Bridge denomination: %d sats", denomination)

        baseline = get_account_balance(rpc, account_id_hex)
        logger.info("Starting balance: %d sats", baseline)

        # Step 1: fund the account through the real bridge.
        for offset in range(DEPOSIT_COUNT):
            dt_index = FIRST_DT_INDEX + offset
            drt_txid, dt_txid, _ = submit_real_bridge_deposit(
                btc_rpc,
                operator_xprivs_hex=operator_xprivs,
                alpen_address_hex=DEPOSIT_SUBJECT_HEX,
                dt_index=dt_index,
                account_serial=TEST_ACCOUNT_SERIAL,
            )
            logger.info(
                "Deposit %d/%d submitted dt_index=%d drt=%s dt=%s",
                offset + 1,
                DEPOSIT_COUNT,
                dt_index,
                drt_txid,
                dt_txid,
            )
            # Mine past the reorg-safe depth, then cross an epoch boundary so
            # the deposit-bearing manifest is folded into OL state.
            btc_rpc.proxy.generatetoaddress(8, miner_addr)
            strata.wait_for_additional_blocks(2 * slots_per_epoch, rpc, timeout_per_block=15)

        # Step 2: assert the deposits credited.
        deposited_total = DEPOSIT_COUNT * denomination
        expected_after_deposit = baseline + deposited_total
        balance = wait_until_with_value(
            lambda: get_account_balance(rpc, account_id_hex),
            lambda b: b == expected_after_deposit,
            error_with=(
                f"account {account_id_hex} not credited with {deposited_total} sats "
                f"(expected total {expected_after_deposit})"
            ),
            timeout=120,
        )
        logger.info("Balance after deposits: %d sats (+%d)", balance, deposited_total)

        # Step 3: build the withdrawal from current snark account state.
        account_state = rpc.strata_getSnarkAccountStateByTag(account_id_hex, "latest")
        if account_state is None:
            raise AssertionError("Account state not found")

        seq_no = account_state["seq_no"]
        next_inbox_idx = account_state["next_inbox_msg_idx"]
        inner_state_hex = account_state["inner_state"]

        withdrawal_dest = b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        dest_hex = withdrawal_dest.hex()

        logger.info("Building withdrawal: %d sats", denomination)
        tx_json = build_snark_withdrawal(
            target_hex=account_id_hex,
            seq_no=seq_no,
            inner_state_hex=inner_state_hex,
            next_inbox_idx=next_inbox_idx,
            dest_hex=dest_hex,
            amount=denomination,
            fees=0,
        )
        logger.info(f"Built withdrawal tx: {tx_json}")

        # Step 4: submit it.
        logger.info("Submitting withdrawal transaction...")
        tx_id = submit_rpc.strata_submitTransaction(tx_json)
        logger.info(f"Withdrawal submitted, ID: {tx_id}")

        strata.wait_for_additional_blocks(2, rpc, timeout_per_block=15)

        # Step 5: assert the withdrawal debited exactly one denomination.
        final_balance = get_account_balance(rpc, account_id_hex)
        expected_balance = expected_after_deposit - denomination

        logger.info(f"Balance: {balance} -> {final_balance} (expected: {expected_balance})")

        if final_balance != expected_balance:
            raise AssertionError(
                f"Balance mismatch after withdrawal: "
                f"expected {expected_balance}, got {final_balance}"
            )

        logger.info("Deposit + withdrawal test passed!")
        return True

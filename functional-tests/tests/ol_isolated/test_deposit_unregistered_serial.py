"""
Regression test: a deposit whose descriptor encodes an unregistered account
serial must not credit any account.

Background. The alpen-cli historically encoded `AccountSerial::zero()` in
the deposit descriptor (see `bin/alpen-cli/src/cmd/deposit.rs`). Serial 0
falls inside the system-reserved range (0..128) and resolves to the
placeholder `AccountId::zero()` rather than to a real account. The OL
then silently discards the funds inside `account_processing::process_message`
(the target-does-not-exist branch).

This test injects a mock deposit with `account_serial=0` via the debug
subprotocol. That path goes through the same `process_asm_log -> process_deposit_log
-> process_message` chain the real bridge uses, so it reproduces the on-chain
failure deterministically without standing up bridge operators.

We assert the registered test account at serial 128 is not credited. Once
the alpen-cli is fixed (separate commit in this PR) the wallet stops
encoding serial=0, but this test guards the OL against silently re-introducing
the same loss-of-funds path.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)

# Test account in the `ol_isolated` env. System serials occupy 0..128, so the
# first user account registered at genesis lands at serial 128.
TEST_ACCOUNT_REF = 0x42
TEST_ACCOUNT_ID_HEX = "00" * 31 + f"{TEST_ACCOUNT_REF:02x}"
TEST_ACCOUNT_SERIAL = 128

# The buggy descriptor serial we are reproducing.
UNREGISTERED_SERIAL = 0

DEPOSIT_AMOUNT_SATS = 1_000_000_000


def get_account_balance(rpc, account_id_hex: str) -> int:
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    if not summaries:
        return 0
    return summaries[0]["balance"]


@flexitest.register
class TestDepositUnregisteredSerial(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("ol_isolated")

    def main(self, ctx):
        from common.test_cli import create_mock_deposit

        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        initial = get_account_balance(rpc, TEST_ACCOUNT_ID_HEX)
        if initial != 0:
            raise AssertionError(f"expected 0 starting balance, got {initial}")

        logger.info(
            "injecting deposit with unregistered serial=%d amount=%d",
            UNREGISTERED_SERIAL,
            DEPOSIT_AMOUNT_SATS,
        )
        txid = create_mock_deposit(
            account_serial=UNREGISTERED_SERIAL,
            amount=DEPOSIT_AMOUNT_SATS,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        logger.info("mock deposit broadcast txid=%s", txid)

        addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(8, addr)

        # Cross at least one terminal block so the deposit-bearing manifest is
        # processed by the OL STF.
        slots_per_epoch = strata.props["slots_per_epoch"]
        strata.wait_for_additional_blocks(2 * slots_per_epoch, rpc, timeout_per_block=15)

        balance = get_account_balance(rpc, TEST_ACCOUNT_ID_HEX)
        if balance != 0:
            raise AssertionError(
                f"deposit with unregistered serial should not credit any account, "
                f"but registered account at serial {TEST_ACCOUNT_SERIAL} has balance {balance}"
            )

        logger.info("deposit with serial=0 was correctly not credited")
        return True

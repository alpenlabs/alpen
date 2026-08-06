"""
Regression test: a deposit whose descriptor encodes the wrong account serial
must not credit the registered Alpen EE test account.

Background. The alpen-cli historically encoded `AccountSerial::zero()` in
the deposit descriptor (see `bin/alpen-cli/src/cmd/deposit.rs`). Serial 0
falls inside the system-reserved range (0..128) and resolves to no account
at all, so the OL sweeps the funds to limbo via the unknown-serial branch
in `resolve_deposit_destination` (`crates/ol/stf/src/manifest_processing.rs`).

Both deposits here go through the real bridge subprotocol. The bridge treats
the destination descriptor as opaque — it validates the DT lock script, the
DRT script and the amount, then emits a `DepositLog` carrying the descriptor
verbatim — so a serial-0 descriptor reaches exactly the OL branch under test.

The test is deliberately structured so it cannot pass vacuously. A deposit
to the *registered* serial runs first and must credit; if the injection
pipeline ever silently stops working, that positive control fails rather
than the serial-0 assertion passing for the wrong reason. We additionally
assert the OL logged the unknown-serial limbo warning for the bad deposit.
"""

import logging
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.bridge import read_bridge_denomination, read_operator_xprivs, submit_real_bridge_deposit
from common.config import ServiceType
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# Test account in the `ol_isolated` env. System serials occupy 0..128, so the
# first user account registered at genesis lands at serial 128.
TEST_ACCOUNT_REF = 0x42
TEST_ACCOUNT_ID_HEX = "00" * 31 + f"{TEST_ACCOUNT_REF:02x}"
TEST_ACCOUNT_SERIAL = 128

# The buggy descriptor serial we are reproducing.
UNREGISTERED_SERIAL = 0

# Bridge deposit indices must be unique across the env. `test_mock_withdrawal`
# shares this environment and uses the low indices, so start well clear of it
# to keep the two tests order-independent.
POSITIVE_CONTROL_DT_INDEX = 10
UNREGISTERED_DT_INDEX = 11

# Destination subject is opaque to balance crediting — only the serial in the
# descriptor selects the account.
DEPOSIT_SUBJECT_HEX = "00" * 20

# The OL emits this when a deposit names a serial with no account behind it.
# Kept in sync with `resolve_deposit_destination` in
# `crates/ol/stf/src/manifest_processing.rs`.
LIMBO_UNKNOWN_SERIAL_LOG = "limboing deposit for unknown account serial"


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
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()
        miner_addr = btc_rpc.proxy.getnewaddress()

        operator_xprivs = read_operator_xprivs(strata)
        denomination = read_bridge_denomination(strata)
        logger.info("bridge denomination: %d sats", denomination)

        baseline = get_account_balance(rpc, TEST_ACCOUNT_ID_HEX)
        logger.info("starting balance for account %s: %d sats", TEST_ACCOUNT_ID_HEX, baseline)

        # Positive control: a deposit naming the registered serial must credit.
        # This is what keeps the test honest — without it, a dead deposit
        # pipeline would make the assertion below pass for the wrong reason.
        logger.info("submitting positive-control deposit to serial %d", TEST_ACCOUNT_SERIAL)
        self._submit_deposit(
            btc_rpc=btc_rpc,
            strata=strata,
            rpc=rpc,
            miner_addr=miner_addr,
            operator_xprivs=operator_xprivs,
            account_serial=TEST_ACCOUNT_SERIAL,
            dt_index=POSITIVE_CONTROL_DT_INDEX,
        )

        expected = baseline + denomination
        wait_until_with_value(
            lambda: get_account_balance(rpc, TEST_ACCOUNT_ID_HEX),
            lambda b: b == expected,
            error_with=(
                f"positive-control deposit did not credit account {TEST_ACCOUNT_ID_HEX}: "
                f"expected {expected} sats"
            ),
            timeout=120,
        )
        logger.info("positive control credited: %d -> %d sats", baseline, expected)

        # Now the regression itself. Record where the log ends so we only scan
        # what this deposit produces.
        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size if log_path.exists() else 0

        logger.info("submitting deposit with unregistered serial=%d", UNREGISTERED_SERIAL)
        self._submit_deposit(
            btc_rpc=btc_rpc,
            strata=strata,
            rpc=rpc,
            miner_addr=miner_addr,
            operator_xprivs=operator_xprivs,
            account_serial=UNREGISTERED_SERIAL,
            dt_index=UNREGISTERED_DT_INDEX,
        )

        # Prove the deposit actually reached the OL branch under test, rather
        # than being dropped somewhere upstream (which would also leave the
        # balance unchanged, but for the wrong reason).
        wait_until_with_value(
            lambda: self._log_tail(log_path, log_offset),
            lambda tail: LIMBO_UNKNOWN_SERIAL_LOG in tail,
            error_with=(
                f"OL never logged {LIMBO_UNKNOWN_SERIAL_LOG!r} for the serial-"
                f"{UNREGISTERED_SERIAL} deposit; it may not have reached "
                "`resolve_deposit_destination` at all"
            ),
            timeout=120,
        )
        logger.info("OL swept the serial-%d deposit to limbo", UNREGISTERED_SERIAL)

        balance = get_account_balance(rpc, TEST_ACCOUNT_ID_HEX)
        if balance != expected:
            raise AssertionError(
                "deposit with unregistered serial must not credit the registered test account, "
                f"but account at serial {TEST_ACCOUNT_SERIAL} moved from {expected} to {balance} "
                f"sats (delta {balance - expected})"
            )

        logger.info(
            "deposit with serial=%d did not credit the registered test account (balance stayed %d)",
            UNREGISTERED_SERIAL,
            balance,
        )
        return True

    @staticmethod
    def _submit_deposit(
        btc_rpc,
        strata,
        rpc,
        miner_addr: str,
        operator_xprivs: list[str],
        account_serial: int,
        dt_index: int,
    ) -> None:
        """Broadcast a real bridge deposit and mine it deep enough for ASM.

        The deposit-bearing manifest is only folded into OL state when an
        epoch closes after the deposit lands on L1, so this mines past the
        reorg-safe depth and then waits out an epoch boundary.
        """
        drt_txid, dt_txid, _ = submit_real_bridge_deposit(
            btc_rpc,
            operator_xprivs_hex=operator_xprivs,
            alpen_address_hex=DEPOSIT_SUBJECT_HEX,
            dt_index=dt_index,
            account_serial=account_serial,
        )
        logger.info(
            "deposit submitted serial=%d dt_index=%d drt=%s dt=%s",
            account_serial,
            dt_index,
            drt_txid,
            dt_txid,
        )

        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        slots_per_epoch = strata.props["slots_per_epoch"]
        strata.wait_for_additional_blocks(2 * slots_per_epoch, rpc, timeout_per_block=15)

    @staticmethod
    def _log_tail(log_path: Path, offset: int) -> str:
        if not log_path.exists():
            return ""
        with log_path.open("r", errors="replace") as handle:
            handle.seek(offset)
            return handle.read()

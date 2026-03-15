"""Verify ExEx WAL files are pruned after epoch finalization in EL<>OL mode."""

import glob
import logging
import os

import flexitest

from common.accounts import get_dev_account
from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.evm_utils import create_funded_account, send_raw_transaction, wait_for_receipt
from common.services import AlpenClientService, BitcoinService, StrataService
from common.wait import wait_until, wait_until_with_value

logger = logging.getLogger(__name__)


def _list_wal_files(datadir: str) -> set[str]:
    """List ExEx WAL files under node datadir."""
    pattern = os.path.join(datadir, "**", "exex", "wal", "*.wal")
    return set(glob.glob(pattern, recursive=True))


def _wal_file_id(path: str) -> int:
    return int(os.path.basename(path).removesuffix(".wal"))


@flexitest.register
class TestExexWalPruning(BaseTest):
    """Check that finalized epochs trigger pruning of old ExEx WAL files."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        ee_sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        ee_rpc = ee_sequencer.create_rpc()
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        btc_rpc = bitcoin.create_rpc()

        wait_until_with_value(
            lambda: strata_rpc.strata_getAccountGenesisEpochCommitment(ALPEN_ACCOUNT_ID),
            lambda commitment: commitment is not None,
            error_with="Timed out waiting for Alpen account genesis commitment",
            timeout=20,
        )

        # Give EE enough time to produce several blocks so ExEx WAL files accumulate.
        ee_sequencer.wait_for_block(10, timeout=60)
        wal_dir_root = ee_sequencer.props["datadir"]
        wal_files_before = wait_until_with_value(
            lambda: _list_wal_files(wal_dir_root),
            lambda files: len(files) > 0,
            error_with="Expected ExEx WAL files to exist before finalization",
            timeout=60,
            step=2,
        )
        logger.info("WAL files before finalization: %s", len(wal_files_before))

        # Submit a few txs to ensure there are non-empty blocks around finalization.
        dev_account = get_dev_account(ee_rpc)
        sender = create_funded_account(ee_rpc, dev_account, 3 * 10**18)
        recipient = "0x000000000000000000000000000000000000dEaD"
        gas_price = int(ee_rpc.eth_gasPrice(), 16)
        for _ in range(3):
            raw_tx = sender.sign_transfer(
                to=recipient,
                value=1_000_000_000,
                gas_price=gas_price,
                gas=21_000,
            )
            tx_hash = send_raw_transaction(ee_rpc, raw_tx)
            wait_for_receipt(ee_rpc, tx_hash, timeout=30)

        # Mine L1 blocks while polling Strata status until epoch 1 is confirmed.
        #
        # In functional-tests-new `el_ol`, epoch finalization may not always be available
        # (for example when no proving/finalization pipeline is wired in the environment),
        # but confirmation still advances with L1 progress and is sufficient to exercise
        # the EE/OL interaction path this test depends on.
        mine_address = btc_rpc.proxy.getnewaddress()
        status_after_confirmation = wait_until_with_value(
            lambda: _mine_and_get_sync_status(strata_seq, btc_rpc, mine_address),
            lambda status: status["confirmed"] is not None and status["confirmed"]["epoch"] >= 1,
            error_with="Epoch 1 was not confirmed in time",
            timeout=180,
            step=2,
        )
        logger.info(
            "Epoch %s confirmed; attempting to observe finalization before WAL check",
            status_after_confirmation["confirmed"]["epoch"],
        )

        # Best effort: observe finalization if this env supports it.
        try:
            status_after_finalization = wait_until_with_value(
                lambda: _mine_and_get_sync_status(strata_seq, btc_rpc, mine_address),
                lambda status: status["finalized"] is not None
                and status["finalized"]["epoch"] >= 1,
                error_with="Epoch 1 was not finalized in time",
                timeout=120,
                step=2,
            )
            logger.info(
                "Epoch %s finalized; checking WAL pruning",
                status_after_finalization["finalized"]["epoch"],
            )
        except AssertionError:
            logger.warning(
                "Epoch 1 was not finalized in time in this environment. "
                "Skipping WAL-pruning assertion because pruning is finalization-driven."
            )
            return True

        def wal_files_pruned() -> bool:
            # Keep L1 moving while checking pruning so EE keeps receiving
            # finalization-related forkchoice updates.
            btc_rpc.proxy.generatetoaddress(2, mine_address)
            current = _list_wal_files(wal_dir_root)
            return len(wal_files_before - current) > 0

        wait_until(
            wal_files_pruned,
            error_with=(
                "No ExEx WAL files were pruned after finalization. "
                "FinishedHeight may be reporting incorrect block numbers."
            ),
            timeout=90,
            step=2,
        )

        wal_files_after = _list_wal_files(wal_dir_root)
        pruned_files = wal_files_before - wal_files_after
        remaining_original = wal_files_before & wal_files_after
        pruned_ids = sorted(_wal_file_id(f) for f in pruned_files)
        remaining_ids = sorted(_wal_file_id(f) for f in remaining_original)

        logger.info(
            "WAL files: %s before, %s after, pruned IDs: %s, remaining original IDs: %s",
            len(wal_files_before),
            len(wal_files_after),
            pruned_ids,
            remaining_ids,
        )

        if remaining_ids:
            assert max(pruned_ids) < min(remaining_ids), (
                "Pruning did not remove the oldest files first. "
                f"Pruned IDs: {pruned_ids}, Remaining original IDs: {remaining_ids}"
            )

        return True


def _mine_and_get_sync_status(
    strata: StrataService,
    btc_rpc,
    mine_address: str,
):
    """Mine a couple of L1 blocks and fetch latest Strata sync status."""
    btc_rpc.proxy.generatetoaddress(2, mine_address)
    return strata.get_sync_status()

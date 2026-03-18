"""Verify ExEx WAL files are pruned in EL<>OL mode."""

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
    """Check that ExEx eventually prunes old WAL files."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        ee_sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        ee_rpc = ee_sequencer.create_rpc()
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        btc_rpc = bitcoin.create_rpc()

        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=20,
        )

        # Give EE enough time to produce several blocks so ExEx WAL files accumulate.
        ee_sequencer.wait_for_block(10, timeout=60)
        wal_dir_root = ee_sequencer.props["datadir"]
        wal_files_before = wait_until_with_value(
            lambda: _list_wal_files(wal_dir_root),
            lambda files: len(files) > 0,
            error_with="Expected ExEx WAL files to exist before pruning check",
            timeout=60,
            step=2,
        )
        logger.info("Captured %s WAL files before pruning check", len(wal_files_before))

        # Submit a few txs to ensure there are non-empty blocks while pruning is observed.
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

        mine_address = btc_rpc.proxy.getnewaddress()

        def wal_files_pruned() -> bool:
            # Track only files from the original snapshot. New WAL files can appear while
            # blocks are still being produced, so a shrinking total file count would be a
            # flaky signal. A set difference tells us whether any preexisting file was
            # deleted regardless of how many new files were created meanwhile.
            btc_rpc.proxy.generatetoaddress(2, mine_address)
            current = _list_wal_files(wal_dir_root)
            pruned_files = wal_files_before - current
            if pruned_files:
                logger.info(
                    "Observed pruning of WAL IDs: %s",
                    sorted(_wal_file_id(path) for path in pruned_files),
                )
            return bool(pruned_files)

        wait_until(
            wal_files_pruned,
            error_with=(
                "No preexisting ExEx WAL files were pruned while L1 kept advancing. "
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

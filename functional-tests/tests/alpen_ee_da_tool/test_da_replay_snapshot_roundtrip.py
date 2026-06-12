"""Verifies alpen-ee-da-tool can export and import EE DA replay snapshots."""

import logging
from pathlib import Path

import flexitest

from common.config.constants import DEV_RECIPIENT_ADDRESS
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.wait import timeout_for_expected_blocks, wait_until
from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    run_alpen_ee_da_tool_json,
    wait_for_expected_root_report,
    wait_for_reconstructed_real_da_report_window,
    write_reconstruction_config,
    write_verification_config,
)

logger = logging.getLogger(__name__)


@flexitest.register
class AlpenEeDaToolDaReplaySnapshotRoundtripTest(AlpenEeDaToolTestBase):
    """Checks a genesis EE DA replay snapshot can seed a later DA replay."""

    BATCH_SEALING_BLOCK_COUNT = 3

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        eth_rpc = sequencer.create_rpc()
        discovery_config_path = write_reconstruction_config(bitcoin, sequencer)
        verification_config_path = write_verification_config(bitcoin, sequencer)
        genesis_l1_height = sequencer.props["genesis_l1_height"]
        assert genesis_l1_height is not None and genesis_l1_height > 1

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        first_tx_hash = send_eth_transfer(eth_rpc, nonce, DEV_RECIPIENT_ADDRESS, 10**18)

        def wait_for_transfer(tx_hash: str) -> int:
            tx_block = None

            def transfer_confirmed():
                nonlocal tx_block
                receipt = eth_rpc.eth_getTransactionReceipt(tx_hash)
                if receipt is None:
                    return False
                assert int(receipt.get("status", "0x1"), 16) == 1, (
                    f"transfer {tx_hash} failed with receipt {receipt}"
                )
                tx_block = int(receipt["blockNumber"], 16)
                return True

            wait_until(
                transfer_confirmed,
                error_with=f"ETH transfer {tx_hash} was not confirmed before DA polling",
                timeout=timeout_for_expected_blocks(10, seconds_per_block=15.0, slack_seconds=60),
                step=0.5,
            )
            assert tx_block is not None
            return tx_block

        first_tx_block = wait_for_transfer(first_tx_hash)

        sequencer.advance_to_next_da_window(
            additional_blocks=10,
            timeout_per_block=15.0,
            timeout_slack=60,
        )

        first_window = wait_for_reconstructed_real_da_report_window(
            bitcoin,
            discovery_config_path,
            genesis_l1_height,
            min_last_block_num=first_tx_block,
            poll_attempts=20,
            blocks_per_poll=3,
            safe_depth=2,
            timeout=180,
        )
        first_scan_report = first_window.report
        first_applied_range = first_scan_report["applied_range"]
        assert first_scan_report["replay_start"] == "genesis"

        da_snapshot_path = Path(bitcoin.props["datadir"]) / "alpen-ee-da-tool-da-snapshot.json"
        prefix_report = wait_for_expected_root_report(
            bitcoin,
            sequencer,
            verification_config_path,
            genesis_l1_height,
            first_window.end_height,
            export_snapshot=da_snapshot_path,
            blocks_per_poll=3,
            timeout=180,
        )
        assert prefix_report["replay_start"] == "genesis"
        assert (
            prefix_report["reconstructed_state_root"]
            == first_scan_report["reconstructed_state_root"]
        )
        assert prefix_report["expected_state_root"] == prefix_report["reconstructed_state_root"]
        assert prefix_report["state_root_matches_expected"] is True
        assert da_snapshot_path.exists()

        second_tx_hash = send_eth_transfer(eth_rpc, nonce + 1, DEV_RECIPIENT_ADDRESS, 10**18)
        second_tx_block = wait_for_transfer(second_tx_hash)

        sequencer.advance_to_next_da_window(
            additional_blocks=10,
            timeout_per_block=15.0,
            timeout_slack=60,
        )

        full_window = wait_for_reconstructed_real_da_report_window(
            bitcoin,
            discovery_config_path,
            genesis_l1_height,
            min_last_block_num=second_tx_block,
            min_blob_count=int(first_scan_report["blobs_reassembled"]) + 1,
            poll_attempts=20,
            blocks_per_poll=3,
            safe_depth=2,
            timeout=180,
        )
        full_scan_report = full_window.report
        assert full_scan_report["replay_start"] == "genesis"
        assert full_window.end_height > first_window.end_height

        full_report = wait_for_expected_root_report(
            bitcoin,
            sequencer,
            verification_config_path,
            genesis_l1_height,
            full_window.end_height,
            blocks_per_poll=3,
            timeout=180,
        )
        assert (
            full_report["reconstructed_state_root"] == full_scan_report["reconstructed_state_root"]
        )
        assert full_report["expected_state_root"] == full_report["reconstructed_state_root"]
        assert full_report["state_root_matches_expected"] is True

        split_code, split_scan_report, split_stderr = run_alpen_ee_da_tool_json(
            discovery_config_path,
            first_window.end_height + 1,
            full_window.end_height,
            snapshot=da_snapshot_path,
            timeout=180,
        )
        assert split_code == 0, split_stderr
        assert split_scan_report["replay_start"] == "snapshot"
        assert int(split_scan_report["blobs_reassembled"]) >= 1
        assert split_scan_report["applied_range"] is not None
        assert (
            int(split_scan_report["applied_range"]["first_update_seq_no"])
            == int(first_applied_range["last_update_seq_no"]) + 1
        )
        assert (
            split_scan_report["reconstructed_state_root"] == full_report["reconstructed_state_root"]
        )

        split_report = wait_for_expected_root_report(
            bitcoin,
            sequencer,
            verification_config_path,
            first_window.end_height + 1,
            full_window.end_height,
            snapshot=da_snapshot_path,
            blocks_per_poll=3,
            timeout=180,
        )
        assert split_report["replay_start"] == "snapshot"
        assert split_report["reconstructed_state_root"] == full_report["reconstructed_state_root"]
        assert split_report["expected_state_root"] == split_report["reconstructed_state_root"]
        assert split_report["state_root_matches_expected"] is True

        logger.info(
            "EE DA replay split [%s, %s] + [%s, %s] matched full replay root",
            genesis_l1_height,
            first_window.end_height,
            first_window.end_height + 1,
            full_window.end_height,
        )
        return True

"""Verifies alpen-ee-da-tool reconstructs real DA posted by alpen-client."""

import logging

import flexitest

from common.config.constants import DEV_RECIPIENT_ADDRESS
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.wait import timeout_for_expected_blocks, wait_until
from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    assert_published_inner_root_match,
    run_alpen_ee_da_tool_json,
    wait_for_published_inner_root_report,
    wait_for_reconstructed_real_da_report_window,
    write_reconstruction_config,
    write_verification_config,
)

logger = logging.getLogger(__name__)


@flexitest.register
class AlpenEeDaToolRealDaSuccessTest(AlpenEeDaToolTestBase):
    """Checks the tool can reconstruct and replay production EE DA from L1."""

    BATCH_SEALING_BLOCK_COUNT = 3

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        btc_rpc = bitcoin.create_rpc()
        eth_rpc = sequencer.create_rpc()
        discovery_config_path = write_reconstruction_config(bitcoin, sequencer)
        verification_config_path = write_verification_config(bitcoin, sequencer)
        genesis_l1_height = sequencer.props["genesis_l1_height"]
        assert genesis_l1_height is not None

        nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        tx_hashes = [
            send_eth_transfer(eth_rpc, nonce + idx, DEV_RECIPIENT_ADDRESS, 10**18)
            for idx in range(3)
        ]

        tx_blocks: dict[str, int] = {}

        def all_transfers_confirmed():
            for tx_hash in tx_hashes:
                if tx_hash in tx_blocks:
                    continue
                receipt = eth_rpc.eth_getTransactionReceipt(tx_hash)
                if receipt is None:
                    return False
                assert int(receipt.get("status", "0x1"), 16) == 1, (
                    f"transfer {tx_hash} failed with receipt {receipt}"
                )
                tx_blocks[tx_hash] = int(receipt["blockNumber"], 16)
            return len(tx_blocks) == len(tx_hashes)

        wait_until(
            all_transfers_confirmed,
            error_with="ETH transfers were not confirmed before DA polling",
            timeout=timeout_for_expected_blocks(10, seconds_per_block=15.0, slack_seconds=60),
            step=0.5,
        )
        max_transfer_block = max(tx_blocks.values())
        logger.info("Transfers confirmed through EVM block %s", max_transfer_block)

        sequencer.advance_to_next_da_window(
            additional_blocks=10,
            timeout_per_block=15.0,
            timeout_slack=60,
        )

        da_window = wait_for_reconstructed_real_da_report_window(
            bitcoin,
            discovery_config_path,
            genesis_l1_height,
            min_last_block_num=max_transfer_block,
            poll_attempts=20,
            blocks_per_poll=3,
            safe_depth=2,
            custom_chain=sequencer.props["chain_spec"],
            timeout=180,
        )
        scan_report = da_window.report

        applied_range = scan_report["applied_range"]
        assert scan_report["replay_start"] == "genesis"
        assert int(scan_report["envelope_count"]) >= 1
        assert int(scan_report["blobs_reassembled"]) >= 1
        assert int(applied_range["first_update_seq_no"]) == 0
        assert int(applied_range["last_block_num"]) >= max_transfer_block

        report = wait_for_published_inner_root_report(
            bitcoin,
            sequencer,
            verification_config_path,
            genesis_l1_height,
            da_window.end_height,
            blocks_per_poll=3,
            timeout=180,
        )
        assert report["reconstructed_state_root"] == scan_report["reconstructed_state_root"]
        assert_published_inner_root_match(report)

        wrong_expected_root_hex = "11" * 32
        if wrong_expected_root_hex == report["reconstructed_state_root"]:
            wrong_expected_root_hex = "22" * 32
        wrong_expected_root = "0x" + wrong_expected_root_hex
        code, mismatch_report, stderr = run_alpen_ee_da_tool_json(
            discovery_config_path,
            genesis_l1_height,
            da_window.end_height,
            expected_root=wrong_expected_root,
            custom_chain=sequencer.props["chain_spec"],
            timeout=180,
        )
        assert code == 0, stderr
        assert mismatch_report["expected_state_root"] == wrong_expected_root_hex
        assert mismatch_report["state_root_matches_expected"] is False

        # Keep the mined height observable for debugging without using Python
        # as the reconstruction oracle.
        logger.info("Tool reconstructed DA through L1 height %s", btc_rpc.proxy.getblockcount())
        return True

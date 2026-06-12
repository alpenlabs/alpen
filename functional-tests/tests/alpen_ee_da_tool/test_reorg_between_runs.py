"""Verifies alpen-ee-da-tool rereads the canonical L1 chain for a bounded range."""

import flexitest

from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    INJECT_MAGIC,
    TEST_ENVELOPE_PUBKEY,
    mine_empty_blocks,
    post_envelope_in_one_block,
    run_alpen_ee_da_tool,
    run_alpen_ee_da_tool_json,
    write_reconstruction_config,
)


@flexitest.register
class AlpenEeDaToolReorgBetweenRunsTest(AlpenEeDaToolTestBase):
    """Invalidates a DA-bearing block and verifies the same height range changes."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_reconstruction_config(
            bitcoin,
            sequencer,
            magic_bytes_override=INJECT_MAGIC,
            sequencer_pubkey_override=TEST_ENVELOPE_PUBKEY,
        )
        window, _envelope = post_envelope_in_one_block(
            bitcoin,
            chunks=[b"not a strata-codec DA blob"],
        )

        first_code, _first_stdout, first_stderr = run_alpen_ee_da_tool(
            config_path,
            window.start_height,
            window.end_height,
            timeout=120,
        )
        assert first_code == 1, first_stderr

        btc_rpc = bitcoin.create_rpc()
        invalidate_hash = btc_rpc.proxy.getblockhash(window.start_height)
        btc_rpc.proxy.invalidateblock(invalidate_hash)
        mine_empty_blocks(bitcoin, window.end_height - window.start_height + 1)

        second_code, second_report, second_stderr = run_alpen_ee_da_tool_json(
            config_path,
            window.start_height,
            window.end_height,
            timeout=120,
        )
        assert second_code == 0, second_stderr
        assert second_report["replay_start"] == "genesis"
        assert second_report.get("envelope_count") == 0
        assert second_report.get("blobs_reassembled") == 0
        assert second_report.get("applied_range") is None
        return True

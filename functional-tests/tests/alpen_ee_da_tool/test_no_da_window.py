"""Verifies alpen-ee-da-tool exits cleanly for a confirmed window with no EE DA."""

import flexitest

from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    run_alpen_ee_da_tool_json,
    write_verification_config,
)


@flexitest.register
class AlpenEeDaToolNoDaWindowTest(AlpenEeDaToolTestBase):
    """Ensures empty DA windows return a deterministic zero-blob report."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verification_config(bitcoin, sequencer)
        genesis_l1_height = sequencer.props["genesis_l1_height"]
        assert genesis_l1_height is not None and genesis_l1_height > 1

        code, report, stderr = run_alpen_ee_da_tool_json(
            config_path,
            1,
            genesis_l1_height - 1,
            custom_chain=sequencer.props["chain_spec"],
            timeout=120,
        )
        assert code == 0, stderr
        assert report["replay_start"] == "genesis"
        assert report.get("applied_range") is None
        assert report.get("envelope_count") == 0
        assert report.get("blobs_reassembled") == 0
        assert report.get("expected_state_root") is None
        assert report.get("state_root_matches_expected") is None
        assert report.get("reconstructed_inner_state_root") is None
        assert report.get("expected_inner_state_root") is None
        assert report.get("inner_state_root_matches_expected") is None
        assert report.get("inner_state_root_mismatch_update_seq_no") is None
        return True

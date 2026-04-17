"""Verifies alpen-ee-da-tool exits cleanly for a confirmed window with no EE DA."""

import flexitest

from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    run_alpen_ee_da_tool,
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
            timeout=120,
        )
        assert code == 0, stderr
        assert report["replay_start"] == "genesis"
        assert report.get("applied_range") is None
        assert report.get("envelope_count") == 0
        assert report.get("blobs_reassembled") == 0
        assert report.get("expected_state_root") is None
        assert report.get("state_root_matches_expected") is None

        config_contents = config_path.read_text(encoding="utf-8")
        missing_account_config = config_path.with_name("alpen-ee-da-tool-missing-account.toml")
        missing_account_config.write_text(
            "\n".join(
                line
                for line in config_contents.splitlines()
                if not line.startswith("ee_snark_account_id")
            )
            + "\n",
            encoding="utf-8",
        )
        code, _stdout, stderr = run_alpen_ee_da_tool(
            missing_account_config,
            1,
            genesis_l1_height - 1,
            timeout=120,
        )
        assert code == 1, "expected incomplete expected-root config to fail"
        assert "incomplete expected-root comparison config" in stderr

        missing_ol_url_config = config_path.with_name("alpen-ee-da-tool-missing-ol-url.toml")
        missing_ol_url_config.write_text(
            "\n".join(
                line for line in config_contents.splitlines() if not line.startswith("ol_rpc_url")
            )
            + "\n",
            encoding="utf-8",
        )
        code, _stdout, stderr = run_alpen_ee_da_tool(
            missing_ol_url_config,
            1,
            genesis_l1_height - 1,
            timeout=120,
        )
        assert code == 1, "expected incomplete expected-root config to fail"
        assert "incomplete expected-root comparison config" in stderr
        return True

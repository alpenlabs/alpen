"""Verifies `ee-da-verify` reports expected-root matches for reconstructed DA."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    post_synthetic_da_window,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyExpectedRootMatchTest(EeDaVerifyTestBase):
    """Runs reconstruction twice and uses the first root as `--expected-root`."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=b"TEST")
        window = post_synthetic_da_window(bitcoin, sequencer)

        first_code, first_report, first_stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            timeout=180,
        )
        assert first_code == 0, first_stderr
        assert first_report.get("applied_range") is not None
        assert first_report.get("blobs_reassembled") == 1

        code, report, stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            expected_root=first_report["final_state_root"],
            timeout=180,
        )
        assert code == 0, stderr
        assert report.get("state_root_matches_expected") is True
        return True

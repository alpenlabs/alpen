"""Verifies `ee-da-verify` reports expected-root mismatches without failing the run."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    mutate_root_hex,
    post_synthetic_da_window,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyExpectedRootMismatchTest(EeDaVerifyTestBase):
    """Ensures mismatch is a report field, not a dedicated process exit code."""

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
        different_expected_root = mutate_root_hex(first_report["final_state_root"])

        code, report, stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            expected_root=different_expected_root,
            timeout=180,
        )
        assert code == 0, stderr
        assert report.get("state_root_matches_expected") is False
        return True

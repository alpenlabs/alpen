"""
Verifies `ee-da-verify` reports a mismatch when reconstructed root
differs from `--expected-root`.
"""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    ensure_multi_envelope_window,
    mutate_root_hex,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyExpectedRootMismatchTest(EeDaVerifyTestBase):
    """Ensures mismatch is reported in JSON output for different expected root."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer)
        window, first_report = ensure_multi_envelope_window(
            config_path, sequencer, bitcoin, min_envelope_count=2
        )
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

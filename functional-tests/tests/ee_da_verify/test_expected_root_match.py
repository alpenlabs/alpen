"""Verifies `ee-da-verify` confirms a match when reconstructed root equals `--expected-root`."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    ensure_multi_envelope_window,
    fetch_ee_state_root,
    parse_applied_last_block_num,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyExpectedRootMatchTest(EeDaVerifyTestBase):
    """Ensures `--expected-root` matches when sourced from applied-range EE block."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer)

        window, dry_run_report = ensure_multi_envelope_window(
            config_path,
            sequencer,
            bitcoin,
            min_envelope_count=2,
        )
        last_applied = parse_applied_last_block_num(dry_run_report)
        expected_root = fetch_ee_state_root(sequencer, last_applied)

        code, report, stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            expected_root=expected_root,
            timeout=180,
        )
        assert code == 0, stderr
        assert report.get("state_root_matches_expected") is True
        return True

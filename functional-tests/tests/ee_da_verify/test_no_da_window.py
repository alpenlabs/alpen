"""Verifies `ee-da-verify` exits cleanly with an empty result when the scan window has no DA."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import run_ee_da_verify_json, write_verifier_config


@flexitest.register
class EeDaVerifyNoDaWindowTest(EeDaVerifyTestBase):
    """Ensures empty-DA windows return a deterministic zero-blob result."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer)
        genesis_l1_height = sequencer.props["genesis_l1_height"]
        assert genesis_l1_height is not None and genesis_l1_height > 1
        start_height = 1
        end_height = genesis_l1_height - 1

        # Scan only the pre-DA region (before sequencer DA genesis height).
        code, report, stderr = run_ee_da_verify_json(
            config_path,
            start_height,
            end_height,
            timeout=120,
        )
        assert code == 0, stderr
        assert report.get("applied_range") is None
        assert report.get("blobs_reassembled") == 0
        return True

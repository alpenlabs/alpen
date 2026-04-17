"""Verifies the verifier reads the active L1 chain for a bounded height window."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    post_synthetic_da_window,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyReorgBetweenRunsTest(EeDaVerifyTestBase):
    """A reorg replacing the DA block changes the result for the same heights."""

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

        btc_rpc = bitcoin.create_rpc()
        invalidate_hash = btc_rpc.proxy.getblockhash(window.start_height)
        btc_rpc.proxy.invalidateblock(invalidate_hash)
        mine_address = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(window.end_height - window.start_height + 1, mine_address)

        second_code, second_report, second_stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            timeout=180,
        )
        assert second_code == 0, second_stderr
        assert second_report.get("applied_range") is None
        assert second_report.get("envelope_count") == 0
        assert second_report.get("blobs_reassembled") == 0
        return True

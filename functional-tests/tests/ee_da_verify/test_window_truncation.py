"""Verifies `ee-da-verify` rejects a scan window that omits the reveal chain's head."""

import flexitest

from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes
from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    produce_da_window,
    run_ee_da_verify,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyWindowTruncationTest(EeDaVerifyTestBase):
    """
    Ensures reconstruction fails when scan window omits the head of the reveal chain.

    The test first confirms the full discovered window succeeds, then truncates
    the start height to the second distinct envelope height and expects failure.
    """

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer)
        btc_rpc = bitcoin.create_rpc()
        scan_start_height = btc_rpc.proxy.getblockcount() + 1

        for _ in range(5):
            produce_da_window(
                sequencer,
                bitcoin,
                scan_start_height=scan_start_height,
                min_envelopes=2,
                timeout=300,
            )
            current_tip = btc_rpc.proxy.getblockcount()
            envelopes = scan_for_da_envelopes(btc_rpc, scan_start_height, current_tip)
            distinct_heights = sorted({envelope.height for envelope in envelopes})
            if len(distinct_heights) < 2:
                continue

            full_start = distinct_heights[0]
            full_end = distinct_heights[-1]
            truncated_start = distinct_heights[1]

            full_code, _full_report, full_stderr = run_ee_da_verify_json(
                config_path,
                full_start,
                full_end,
                timeout=240,
            )
            assert full_code == 0, full_stderr

            truncated_code, _truncated_stdout, truncated_stderr = run_ee_da_verify(
                config_path,
                truncated_start,
                full_end,
                timeout=240,
            )
            assert truncated_code == 1
            assert "failed to walk reveal chain" in truncated_stderr
            return True

        raise AssertionError("failed to produce DA envelopes across at least two L1 heights")

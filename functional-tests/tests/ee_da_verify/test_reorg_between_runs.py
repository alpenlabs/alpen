"""Verifies `ee-da-verify` handles an L1 reorg that replaces envelopes seen in a prior run."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    produce_da_window,
    run_ee_da_verify_json,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyReorgBetweenRunsTest(EeDaVerifyTestBase):
    """
    Verifies that the same scan window produces different reconstruction results
    after an L1 reorg replaces DA-containing blocks.
    """

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer)
        window = produce_da_window(sequencer, bitcoin, min_envelopes=2, timeout=300)

        # Baseline run on the pre-reorg active chain.
        first_code, first_report, first_stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            timeout=240,
        )
        assert first_code == 0, first_stderr
        assert first_report.get("applied_range") is not None
        assert int(first_report.get("envelope_count", 0)) >= 1

        # Freeze new DA posting and reorg away the DA-containing segment.
        sequencer.stop()
        btc_rpc = bitcoin.create_rpc()
        invalidate_hash = btc_rpc.proxy.getblockhash(window.start_height)
        btc_rpc.proxy.invalidateblock(invalidate_hash)

        mine_address = btc_rpc.proxy.getnewaddress()
        replacement_block_count = window.end_height - window.start_height + 1
        btc_rpc.proxy.generatetoaddress(replacement_block_count, mine_address)

        # Same height window on the new active chain should no longer replay the
        # original DA payload.
        second_code, second_report, second_stderr = run_ee_da_verify_json(
            config_path,
            window.start_height,
            window.end_height,
            timeout=240,
        )
        assert second_code == 0, second_stderr
        assert second_report.get("applied_range") is None
        assert second_report.get("blobs_reassembled") == 0
        assert second_report.get("final_state_root") != first_report.get("final_state_root")
        return True

"""Verifies alpen-ee-da-tool rejects incomplete confirmed EE DA envelopes."""

import flexitest

from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    INJECT_MAGIC,
    TEST_ENVELOPE_PUBKEY,
    post_envelope_in_one_block,
    run_alpen_ee_da_tool,
    write_reconstruction_config,
)


@flexitest.register
class AlpenEeDaToolMissingRevealTest(AlpenEeDaToolTestBase):
    """Posts a confirmed commit with an unspent reveal slot and expects scan failure."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_reconstruction_config(
            bitcoin,
            sequencer,
            magic_bytes_override=INJECT_MAGIC,
            sequencer_pubkey_override=TEST_ENVELOPE_PUBKEY,
        )
        window, _envelope = post_envelope_in_one_block(
            bitcoin,
            chunks=[b"missing reveal chunk"],
            reveal_count=0,
        )

        code, _stdout, stderr = run_alpen_ee_da_tool(
            config_path,
            window.start_height,
            window.end_height,
            timeout=120,
        )
        assert code == 1, f"expected failure for missing reveal, got {code}"
        assert "missingreveal" in stderr.lower().replace(" ", ""), stderr
        return True

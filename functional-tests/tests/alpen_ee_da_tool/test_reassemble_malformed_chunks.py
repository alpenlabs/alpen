"""Verifies alpen-ee-da-tool reports DA blob reassembly errors."""

import flexitest

from tests.alpen_ee_da_tool.base import AlpenEeDaToolTestBase
from tests.alpen_ee_da_tool.helpers import (
    TEST_ENVELOPE_PUBKEY,
    post_envelope_in_one_block,
    run_alpen_ee_da_tool,
    write_reconstruction_config,
)


@flexitest.register
class AlpenEeDaToolInvalidChunkTest(AlpenEeDaToolTestBase):
    """Posts a complete envelope whose chunk bytes are not a valid encoded DaBlob."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_reconstruction_config(
            bitcoin,
            sequencer,
            sequencer_pubkey_override=TEST_ENVELOPE_PUBKEY,
        )
        window, _envelope = post_envelope_in_one_block(
            bitcoin,
            chunks=[b"not a strata-codec DA blob"],
        )

        code, _stdout, stderr = run_alpen_ee_da_tool(
            config_path,
            window.start_height,
            window.end_height,
            timeout=120,
        )
        assert code == 1, f"expected reassembly failure, got {code}"
        assert "failed to reassemble da blobs" in stderr.lower(), stderr
        return True

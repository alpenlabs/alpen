"""Verifies `ee-da-verify` rejects a window whose first envelope is missing its head."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    INJECT_MAGIC,
    ZERO_PREV_WTXID_HEX,
    craft_chunk_bytes,
    inject_da_window,
    post_ee_da_envelope,
    run_ee_da_verify,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifySegmentStartsMidEnvelopeTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        start, end = inject_da_window(
            bitcoin,
            inject=lambda: post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=[craft_chunk_bytes(b"\x33" * 32, chunk_index=1, total_chunks=2, body=b"x")],
            ),
        )
        code, _stdout, stderr = run_ee_da_verify(config_path, start, end)
        assert code == 1, f"expected exit 1, got {code}. stderr={stderr}"
        assert "ChunkStreamStartsMidEnvelope" in stderr, f"missing variant in stderr: {stderr}"
        return True

"""Verifies `ee-da-verify` rejects an incomplete envelope (fewer chunks landed than declared)."""

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
class EeDaVerifySegmentEndsMidEnvelopeTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        start, end = inject_da_window(
            bitcoin,
            inject=lambda: post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=[
                    craft_chunk_bytes(b"\x88" * 32, chunk_index=0, total_chunks=3, body=b"only")
                ],
            ),
        )
        code, _stdout, stderr = run_ee_da_verify(config_path, start, end)
        assert code == 1, f"expected exit 1, got {code}. stderr={stderr}"
        assert "ChunkStreamEndsMidEnvelope" in stderr, f"missing variant in stderr: {stderr}"
        return True

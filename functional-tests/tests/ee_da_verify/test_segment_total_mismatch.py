"""Verifies `ee-da-verify` rejects chunks of one envelope that disagree on envelope length."""

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
class EeDaVerifySegmentTotalMismatchTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()
        blob_hash = b"\x77" * 32
        chunks = [
            craft_chunk_bytes(blob_hash, 0, total_chunks=3, body=b"a"),
            craft_chunk_bytes(blob_hash, 1, total_chunks=5, body=b"b"),  # total differs
        ]
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        start, end = inject_da_window(
            bitcoin,
            inject=lambda: post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=chunks,
            ),
        )
        code, _stdout, stderr = run_ee_da_verify(config_path, start, end)
        assert code == 1, f"expected exit 1, got {code}. stderr={stderr}"
        assert "TotalChunksMismatch" in stderr, f"missing variant in stderr: {stderr}"
        return True

"""Verifies `ee-da-verify` rejects an envelope with gaps in its chunk sequence."""

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
class EeDaVerifySegmentNonContiguousTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()
        blob_hash = b"\x44" * 32
        total = 4
        chunks = [
            craft_chunk_bytes(blob_hash, 0, total, b"a"),
            craft_chunk_bytes(blob_hash, 1, total, b"b"),
            craft_chunk_bytes(blob_hash, 3, total, b"d"),  # skip index 2
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
        assert "NonContiguousChunkIndex" in stderr, f"missing variant in stderr: {stderr}"
        return True

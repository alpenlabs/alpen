"""Verifies `ee-da-verify` rejects chunks that reassemble into an unparsable blob."""

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
class EeDaVerifyReassembleBadBlobContentsTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()
        # Valid chunk framing + contiguous chain, but the concatenated body
        # is 32 bytes of 0x00 which does not decode as a `DaBlob`.
        blob_hash = b"\x99" * 32
        total = 2
        chunks = [
            craft_chunk_bytes(blob_hash, 0, total, b"\x00" * 16),
            craft_chunk_bytes(blob_hash, 1, total, b"\x00" * 16),
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
        assert "BlobReassembly" in stderr, f"missing variant in stderr: {stderr}"
        return True

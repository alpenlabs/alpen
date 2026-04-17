"""Verifies reassembly-stage malformed DA chunks fail with specific errors."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    INJECT_MAGIC,
    craft_single_chunk_blob,
    encode_synthetic_empty_da_blob,
    inject_da_window,
    post_ee_da_envelope,
    run_ee_da_verify,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyReassembleMalformedChunksTest(EeDaVerifyTestBase):
    """Posts parseable envelopes whose raw chunk payloads violate DA reassembly."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        valid_blob = encode_synthetic_empty_da_blob()
        cases = [
            ("invalid-chunk", [b"ignored"], "OverrunInput"),
            ("none", craft_single_chunk_blob(valid_blob[:16]), "OverrunInput"),
            ("none", craft_single_chunk_blob(valid_blob + b"trailing"), "ExtraInput"),
        ]

        for malformed, chunks, expected_error in cases:
            window = inject_da_window(
                bitcoin,
                lambda malformed=malformed, chunks=chunks: post_ee_da_envelope(
                    bitcoin,
                    sequencer,
                    chunks=chunks,
                    malformed=malformed,
                ),
            )
            code, _stdout, stderr = run_ee_da_verify(
                config_path,
                window.start_height,
                window.end_height,
                timeout=120,
            )
            assert code == 1, f"{expected_error}: expected exit 1, got {code}. stderr={stderr}"
            assert expected_error in stderr, f"missing {expected_error}: {stderr}"
        return True

"""Verifies scan-stage malformed EE-DA envelopes fail with specific errors."""

import flexitest

from tests.ee_da_verify import EeDaVerifyTestBase
from tests.ee_da_verify.helpers import (
    INJECT_MAGIC,
    inject_da_window,
    post_ee_da_envelope,
    run_ee_da_verify,
    write_verifier_config,
)


@flexitest.register
class EeDaVerifyScanMalformedEnvelopesTest(EeDaVerifyTestBase):
    """Posts valid Bitcoin transactions with malformed EE-DA commit/reveal structure."""

    def main(self, ctx):
        bitcoin, sequencer = self._services()
        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        cases = [
            ("unsupported-version", [b"chunk"], "UnsupportedCommitMarkerVersion"),
            ("marker-after-slot", [b"chunk"], "InvalidCommitMarkerOutput"),
            ("multiple-markers", [b"chunk"], "MultipleCommitMarkers"),
            ("missing-reveal-slots", [], "MissingRevealSlots"),
            ("missing-reveal", [b"chunk"], "MissingReveal"),
            ("multi-slot-reveal", [b"chunk-0", b"chunk-1"], "MultipleRevealSlotSpends"),
            ("wrong-sequencer-key", [b"chunk"], "InvalidSequencerPubkey"),
            ("ambiguous-taproot-change", [b"chunk"], "AmbiguousTaprootChangeOutput"),
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
            assert code == 1, f"{malformed}: expected exit 1, got {code}. stderr={stderr}"
            assert expected_error in stderr, f"{malformed}: missing {expected_error}: {stderr}"
        return True

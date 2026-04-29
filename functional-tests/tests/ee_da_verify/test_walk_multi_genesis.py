"""
Verifies `ee-da-verify` rejects a window where multiple envelopes claim
the chain's first position.
"""

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
class EeDaVerifyWalkMultiGenesisTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()

        def inject():
            # Two independent single-chunk envelopes, both anchored at prev=0.
            post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=[craft_chunk_bytes(b"\x11" * 32, 0, 1, b"a")],
            )
            post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=[craft_chunk_bytes(b"\x22" * 32, 0, 1, b"b")],
            )

        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        start, end = inject_da_window(bitcoin, inject=inject)
        code, _stdout, stderr = run_ee_da_verify(config_path, start, end)
        assert code == 1, f"expected exit 1, got {code}. stderr={stderr}"
        assert "MultipleGenesisStarts" in stderr, f"missing variant in stderr: {stderr}"
        return True

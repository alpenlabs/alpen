"""Verifies `ee-da-verify` rejects a forked reveal chain."""

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
class EeDaVerifyWalkForkTest(EeDaVerifyTestBase):
    def main(self, ctx):
        bitcoin, sequencer = self._services()

        def inject():
            a_wtxids = post_ee_da_envelope(
                bitcoin,
                prev_wtxid=ZERO_PREV_WTXID_HEX,
                chunks=[craft_chunk_bytes(b"\x11" * 32, 0, 1, b"a")],
            )
            a_tail = a_wtxids[-1]
            # B and C both chain off A.tail -> two successors for one predecessor.
            post_ee_da_envelope(
                bitcoin,
                prev_wtxid=a_tail,
                chunks=[craft_chunk_bytes(b"\x22" * 32, 0, 1, b"b")],
            )
            post_ee_da_envelope(
                bitcoin,
                prev_wtxid=a_tail,
                chunks=[craft_chunk_bytes(b"\x33" * 32, 0, 1, b"c")],
            )

        config_path = write_verifier_config(bitcoin, sequencer, magic_bytes_override=INJECT_MAGIC)
        start, end = inject_da_window(bitcoin, inject=inject)
        code, _stdout, stderr = run_ee_da_verify(config_path, start, end)
        assert code == 1, f"expected exit 1, got {code}. stderr={stderr}"
        assert "MultipleSuccessors" in stderr, f"missing variant in stderr: {stderr}"
        return True

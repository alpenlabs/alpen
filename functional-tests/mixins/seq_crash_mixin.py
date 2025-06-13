from collections.abc import Callable

import flexitest

from utils import *
from utils.constants import *

from . import BaseMixin


class SeqCrashMixin(BaseMixin):
    """
    Mixin for emulating the crash of sequencer.
    Provides a method for handling the bailout, stops and restarts sequencer under the hood.
    """

    def premain(self, ctx: flexitest.RunContext):
        super().premain(ctx)

        self.debug("checking connectivity")
        protocol_version = self.seqrpc.strata_protocolVersion()
        assert protocol_version is not None, "Sequencer RPC inactive"

    def handle_bail(self, bail_tag: Callable[[], str], **kwargs) -> int:
        """
        Handles the bailout process for the given sequencer RPC.

        Returns the chain_tip_slot before the bailout.
        """
        cur_chain_tip = self.seqrpc.strata_syncStatus()["tip_height"]

        # Trigger the bailout
        self.seqrpc.debug_bail(bail_tag())

        # Ensure the sequencer bails out
        wait_until(
            lambda: check_sequencer_down(self.seqrpc),
            error_with="Sequencer didn't bail out",
            **kwargs,
        )

        # Stop the sequencer to update bookkeeping, we know the sequencer has
        # already stopped
        self.seq.stop()

        # Restart the sequencer
        self.seq.start()

        wait_until(
            lambda: not check_sequencer_down(self.seqrpc),
            error_with="Sequencer didn't start",
            **kwargs,
        )

        return cur_chain_tip

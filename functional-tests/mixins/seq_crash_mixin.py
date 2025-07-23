import time
from abc import abstractmethod
from collections.abc import Callable
from typing import Generic, TypeVar

import flexitest

from utils import *
from utils.constants import *

from . import BaseMixin

T = TypeVar("T")


class SeqCrashMixin(BaseMixin, Generic[T]):
    """
    Mixin for emulating the crash of sequencer.
    Provides a method for handling the bailout, stops and restarts sequencer under the hood.
    """

    def premain(self, ctx: flexitest.RunContext):
        super().premain(ctx)

        self.debug("checking connectivity")
        protocol_version = self.seqrpc.strata_protocolVersion()
        assert protocol_version is not None, "Sequencer RPC inactive"

    @abstractmethod
    def get_recovery_metric(self) -> T:
        """
        Abstract method to get a metric value used to determine crash recovery.
        Derived classes should override this to return their desired recovery metric.
        """
        pass

    def handle_bail(self, bail_tag: Callable[[], str], **kwargs) -> T:
        """
        Handles the bailout process for the given sequencer RPC.

        Returns the recovery metric value before the bailout.
        """
        time.sleep(2)
        recovery_metric = self.get_recovery_metric()

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

        return recovery_metric


class DefaultSeqCrashMixin(SeqCrashMixin[int]):
    """
    Default implementation using syncStatus tip_height as the recovery metric.
    """

    def get_recovery_metric(self) -> int:
        return self.seqrpc.strata_syncStatus()["tip_height"]

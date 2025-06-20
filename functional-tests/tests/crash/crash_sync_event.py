import flexitest

from envs import testenv
from mixins import seq_crash_mixin
from utils import wait_until


@flexitest.register
class CrashSyncEventTest(seq_crash_mixin.SeqCrashMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        cur_chain_tip = self.handle_bail(lambda: "sync_event")

        wait_until(
            lambda: self.seqrpc.strata_clientStatus()["chain_tip_slot"] > cur_chain_tip,
            error_with="chain tip slot not progressing",
            timeout=20,
        )

        return True

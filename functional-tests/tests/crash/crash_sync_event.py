import flexitest

from envs import testenv
from mixins import seq_crash_mixin


@flexitest.register
class CrashSyncEventTest(seq_crash_mixin.SeqCrashMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        cur_chain_tip = self.handle_bail(lambda: "sync_event")

        seq_waiter = self.create_strata_waiter(self.seqrpc)
        seq_waiter.wait_until_chain_tip_exceeds(cur_chain_tip)

        return True

import flexitest

from envs import testenv
from mixins import seq_crash_mixin
from utils import wait_until


@flexitest.register
class CrashFcmNewBlockTest(seq_crash_mixin.DefaultSeqCrashMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        cur_chain_tip = self.handle_bail(lambda: "fcm_new_block")

        wait_until(
            lambda: self.get_recovery_metric() > cur_chain_tip + 1,
            error_with="chain tip slot not progressing",
            timeout=20,
        )

        return True

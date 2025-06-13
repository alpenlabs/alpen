import flexitest

from envs import testenv
from mixins import seq_crash_mixin
from utils import wait_until, wait_until_strata_client_ready


@flexitest.register
class CrashDutySignBlockTest(seq_crash_mixin.SeqCrashMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        wait_until_strata_client_ready(self.seqrpc)

        cur_chain_tip = self.handle_bail(lambda: "duty_sign_block")

        wait_until(
            lambda: self.seqrpc.strata_clientStatus()["chain_tip_slot"] > cur_chain_tip,
            error_with="chain tip slot not progressing",
            timeout=20,
        )

        return True

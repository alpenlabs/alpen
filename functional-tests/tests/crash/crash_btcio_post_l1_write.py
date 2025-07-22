import flexitest

from envs import testenv
from mixins import seq_crash_mixin
from utils import ProverClientSettings, wait_until


@flexitest.register
class CrashBtcioSyncEventTest(seq_crash_mixin.SeqCrashMixin[int]):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_default(),
            )
        )

    def get_recovery_metric(self):
        return self.seqrpc.strata_clientStatus()["tip_l1_block"]["height"]

    def main(self, ctx: flexitest.RunContext):
        cur_tip_block = self.handle_bail(lambda: "btcio_post_l1_write")

        wait_until(
            lambda: self.get_recovery_metric() > cur_tip_block,
            error_with="client state not progressing",
            timeout=20,
        )

        return True

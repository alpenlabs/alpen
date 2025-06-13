import flexitest

from envs import net_settings, testenv
from mixins import seq_crash_mixin
from utils import ProverClientSettings, wait_until, wait_until_strata_client_ready


@flexitest.register
class CrashSyncEventFinalizeEpochTest(seq_crash_mixin.SeqCrashMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        wait_until_strata_client_ready(self.seqrpc)

        cur_chain_tip = self.handle_bail(lambda: "sync_event_finalize_epoch", timeout=60)

        wait_until(
            lambda: self.seqrpc.strata_syncStatus()["tip_height"] > cur_chain_tip + 1,
            error_with="chain tip slot not progressing",
        )

        return True

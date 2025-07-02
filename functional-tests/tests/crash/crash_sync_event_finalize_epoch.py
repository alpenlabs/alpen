import flexitest

from envs import net_settings, testenv
from mixins import seq_crash_mixin
from utils import ProverClientSettings


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
        cur_chain_tip = self.handle_bail(lambda: "sync_event_finalize_epoch", timeout=60)

        seq_waiter = self.create_strata_waiter(self.seqrpc)
        seq_waiter.wait_until_chain_tip_exceeds(cur_chain_tip + 1)

        return True

import flexitest

from envs import testenv
from utils.utils import wait_until_with_value


@flexitest.register
class ExecUpdateTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()
        recent_blks = wait_until_with_value(
            lambda: seqrpc.strata_getRecentBlockHeaders(1),
            lambda value: value is not None,
            error_with="Blocks not generated",
            timeout=2,
        )
        exec_update = seqrpc.strata_getExecUpdateById(recent_blks[0]["block_id"])
        self.debug(exec_update)
        assert exec_update["update_idx"] == recent_blks[0]["block_idx"]

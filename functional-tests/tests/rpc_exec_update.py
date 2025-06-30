import flexitest

from envs import testenv


@flexitest.register
class ExecUpdateTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seq_waiter = self.create_strata_waiter(seq.create_rpc())

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()
        recent_blks = seq_waiter.wait_until_recent_block_headers_at(1)

        exec_update = seqrpc.strata_getExecUpdateById(recent_blks[0]["block_id"])
        self.debug(exec_update)
        assert exec_update["update_idx"] == recent_blks[0]["block_idx"]

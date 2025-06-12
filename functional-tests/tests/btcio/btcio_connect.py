import flexitest

from envs import testenv


@flexitest.register
class L1ConnectTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        seqrpc = seq.create_rpc()
        self.debug("checking connectivity")
        l1stat = seqrpc.strata_l1connected()
        assert l1stat, "Error connecting to Bitcoin Rpc client"

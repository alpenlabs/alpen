import flexitest

from envs import testenv


@flexitest.register
class L1ClientStatusTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        seqrpc = seq.create_rpc()

        proto_ver = seqrpc.strata_protocolVersion()
        self.debug(f"protocol version {proto_ver}")
        assert proto_ver == 1, "query protocol version"

        client_status = seqrpc.strata_clientStatus()
        self.debug(f"client status {client_status}")

        return True

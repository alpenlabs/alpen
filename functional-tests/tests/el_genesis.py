import flexitest

from envs import testenv


@flexitest.register
class ElGenesisTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")

        rethrpc = reth.create_rpc()
        genesis_block = rethrpc.eth_getBlockByNumber(hex(0), True)

        expected = "0x46c0dc60fb131be4ccc55306a345fcc20e44233324950f978ba5f185aa2af4dc"
        assert genesis_block["hash"] == expected, "genesis block hash"

import flexitest

from envs import testenv


@flexitest.register
class ElPendingBlock(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        address = reth.create_web3().address

        rethrpc = reth.create_rpc()
        block = rethrpc.eth_getBlockByNumber("pending", True)

        assert block is not None, "get pending block"

        gas = rethrpc.eth_estimateGas(
            {
                "chainId": "0x3039",
                "from": address,
                "to": "0x" + "00" * 20,
                "nonce": "0x0",
            },
            "pending",
        )

        assert gas is not None, "estimate gas on pending block"

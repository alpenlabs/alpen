import time

import flexitest

from envs import testenv


@flexitest.register
class ElBlockStateDiffDataGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("load_reth")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()
        time.sleep(20)

        block = int(rethrpc.eth_blockNumber(), base=16)
        self.info(f"Latest reth block={block}")

        reconstructed_root = rethrpc.strataee_getStateRootByDiffs(block)
        actual_root = rethrpc.eth_getBlockByNumber(hex(block), False)["stateRoot"]
        self.info(f"reconstructed state root = {reconstructed_root}")
        self.info(f"actual state root = {actual_root}")

        assert reconstructed_root == actual_root, "reconstructured state root is wrong"

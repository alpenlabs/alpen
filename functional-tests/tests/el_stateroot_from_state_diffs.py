import flexitest

from envs import testenv
from utils import get_latest_eth_block_number, wait_until


@flexitest.register
class ElBlockStateDiffDataGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("load_reth")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()

        # Get initial block number and wait for 20 more blocks to be generated
        initial_block = get_latest_eth_block_number(rethrpc)
        wait_until(
            lambda: get_latest_eth_block_number(rethrpc) >= initial_block + 20,
            error_with="Timeout: 20 blocks were not generated",
            timeout=60,
        )

        block = get_latest_eth_block_number(rethrpc)
        self.info(f"Latest reth block={block}")

        reconstructed_root = rethrpc.strataee_getStateRootByDiffs(block)
        actual_root = rethrpc.eth_getBlockByNumber(hex(block), False)["stateRoot"]
        self.info(f"reconstructed state root = {reconstructed_root}")
        self.info(f"actual state root = {actual_root}")

        assert reconstructed_root == actual_root, "reconstructured state root is wrong"

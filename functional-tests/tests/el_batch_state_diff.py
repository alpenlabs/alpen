import flexitest

from envs import testenv
from utils import get_latest_eth_block_number


@flexitest.register
class ElBatchStateDiffTest(testenv.StrataTestBase):
    """Test that getBatchStateDiff correctly aggregates state diffs for a block range."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("load_reth")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc, timeout=60)

        # Wait for some blocks to be generated
        initial_block = get_latest_eth_block_number(rethrpc)
        _ = reth_waiter.wait_until_eth_block_exceeds(
            initial_block + 10,
            message="Timeout: blocks were not generated",
        )

        to_block = get_latest_eth_block_number(rethrpc)
        from_block = 1

        self.info(f"Testing batch state diff for blocks {from_block} to {to_block}")

        # Get the batch state diff
        batch_diff = rethrpc.strataee_getBatchStateDiff(from_block, to_block)
        self.info(f"Batch state diff accounts: {len(batch_diff.get('accounts', {}))}")
        self.info(f"Batch state diff storage: {len(batch_diff.get('storage', {}))}")

        # Validate: state root reconstructed from diffs should match actual
        actual_root = rethrpc.eth_getBlockByNumber(hex(to_block), False)["stateRoot"]
        reconstructed_root = rethrpc.strataee_getStateRootByDiffs(to_block)

        self.info(f"Actual state root: {actual_root}")
        self.info(f"Reconstructed state root: {reconstructed_root}")

        assert actual_root == reconstructed_root, (
            f"State root mismatch! Actual: {actual_root}, Reconstructed: {reconstructed_root}"
        )

        # Validate batch diff is not empty
        assert batch_diff is not None, "Batch diff should not be None"
        assert len(batch_diff.get("accounts", {})) > 0, "Batch diff should have account changes"

        self.info("Batch state diff test passed!")

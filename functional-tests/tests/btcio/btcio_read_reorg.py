import flexitest

from envs import testenv
from envs.testenv import BasicEnvConfig

REORG_DEPTH = 3


@flexitest.register
class L1ReadReorgTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        # standalone env for this test as it involves mutating the blockchain via invalidation
        ctx.set_env(BasicEnvConfig(110))

    def main(self, ctx: flexitest.RunContext):
        # Wait for seq and until l1 reader has enough blocks( > REORG_DEPTH) to be
        # able to reorg properly
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")
        btc_rpc = btc.create_rpc()
        seq_rpc = seq.create_rpc()

        seq_waiter = self.create_strata_waiter(seq_rpc)
        l1_status = seq_waiter.wait_until_l1_height_at(REORG_DEPTH + 1)
        curr_l1_height = l1_status["cur_height"]

        invalidate_height = curr_l1_height - REORG_DEPTH
        self.info(f"height to invalidate from {invalidate_height}")

        block_to_invalidate_from = btc_rpc.proxy.getblockhash(invalidate_height)

        # Invalid block
        self.info(f"invalidating block {block_to_invalidate_from}")
        btc_rpc.proxy.invalidateblock(block_to_invalidate_from)

        to_be_invalid_block = seq_rpc.strata_getL1blockHash(invalidate_height)
        # Wait for at least 1 block to be added after invalidating `REORG_DEPTH` blocks.
        block_from_invalidated_height = seq_waiter.wait_until_l1_height_at(invalidate_height + 1)

        self.info(f"now have block {block_from_invalidated_height}")

        assert to_be_invalid_block != block_from_invalidated_height, (
            f"Expected reorg from block {invalidate_height}"
        )

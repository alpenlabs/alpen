import time

import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from envs.testenv import BasicEnvConfig
from utils.constants import BLOCK_GENERATION_INTERVAL_SECS, SEQ_SLACK_TIME_SECS
from utils.utils import wait_until_with_value

REORG_DEPTH = 3


@flexitest.register
class L1ReadReorgTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        # standalone env for this test as it involves mutating the blockchain via invalidation
        ctx.set_env(BasicEnvConfig(110))

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")

        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc = seq.create_rpc()

        # Wait for seq and until l1 reader has enough blocks( > REORG_DEPTH) to be
        # able to reorg properly
        curr_l1_height = wait_until_with_value(
            lambda: seqrpc.strata_l1status()["cur_height"],
            lambda v: v > REORG_DEPTH,
            error_with="Sequencer did not start on time or does not have enough l1 blocks",
        )

        invalidate_height = curr_l1_height - REORG_DEPTH
        self.info(f"height to invalidate from {invalidate_height}")

        block_to_invalidate_from = btcrpc.proxy.getblockhash(invalidate_height)

        # Invalid block
        self.info(f"invalidating block {block_to_invalidate_from}")
        btcrpc.proxy.invalidateblock(block_to_invalidate_from)

        to_be_invalid_block = seqrpc.strata_getL1blockHash(invalidate_height)
        # Wait for at least 1 block to be added after invalidating `REORG_DEPTH` blocks.
        block_from_invalidated_height = wait_until_with_value(
            lambda: seqrpc.strata_getL1blockHash(invalidate_height + 1),
            lambda value: value is not None,
            error_with="L1 Block not produced in time",
        )

        self.info(f"now have block {block_from_invalidated_height}")

        assert to_be_invalid_block != block_from_invalidated_height, (
            f"Expected reorg from block {invalidate_height}"
        )

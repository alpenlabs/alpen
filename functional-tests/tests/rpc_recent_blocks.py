import flexitest

from envs import testenv
from utils import wait_until

EXPECTED_L2_BLOCKS = 10
BLOCK_NUMBER = 4


@flexitest.register
class RecentBlocksTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()
        wait_until(
            lambda: seqrpc.strata_getHeadersAtIdx(EXPECTED_L2_BLOCKS) is not None,
            error_with=f"Expected block {EXPECTED_L2_BLOCKS} not generated",
            timeout=20,
        )

        recent_blks = seqrpc.strata_getRecentBlockHeaders(EXPECTED_L2_BLOCKS)
        assert len(recent_blks) == EXPECTED_L2_BLOCKS

        # check if they are in order by verifying if N-1 block is parent of N block
        for idx in reversed(range(0, EXPECTED_L2_BLOCKS)):
            if idx != EXPECTED_L2_BLOCKS - 1:
                assert recent_blks[idx]["prev_block"] == recent_blks[idx + 1]["block_id"]

        l2_blk = seqrpc.strata_getHeadersAtIdx(recent_blks[BLOCK_NUMBER]["block_idx"])

        assert recent_blks[BLOCK_NUMBER]["block_idx"] == l2_blk[0]["block_idx"]

        second_blk_from_id = seqrpc.strata_getHeaderById(l2_blk[0]["block_id"])

        # check if we got the correct block when looked using hash
        assert second_blk_from_id["block_id"] == l2_blk[0]["block_id"]

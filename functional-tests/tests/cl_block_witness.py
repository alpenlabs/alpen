import flexitest

from envs import testenv
from utils import wait_until
from utils.utils import wait_until_strata_client_ready

REORG_DEPTH = 3


@flexitest.register
class CLBlockWitnessDataGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()

        wait_until_strata_client_ready(seqrpc)

        witness_1 = self.get_witness(seqrpc, 1)
        assert witness_1 is not None

        wait_until(
            lambda: self.get_witness(seqrpc, 2) is not None,
            error_with="Failed to get cl witness in time",
        )

        return True

    def get_witness(self, seqrpc, idx):
        block_ids = seqrpc.strata_getHeadersAtIdx(idx)
        block_id = block_ids[0]["block_id"]
        witness = seqrpc.strata_getCLBlockWitness(block_id)
        return witness

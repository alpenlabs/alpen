import time

import flexitest

from envs import testenv
from utils import wait_until

REORG_DEPTH = 3


@flexitest.register
class CLBlockWitnessDataGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()

        # Wait for seq
        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="Sequencer did not start on time",
        )

        witness_1 = self.get_witness(seqrpc, 1)
        assert witness_1 is not None

        time.sleep(1)
        witness_2 = self.get_witness(seqrpc, 2)
        assert witness_2 is not None

        return True

    def get_witness(self, seqrpc, idx):
        block_ids = seqrpc.strata_getHeadersAtIdx(idx)
        block_id = block_ids[0]["block_id"]
        witness = seqrpc.strata_getCLBlockWitness(block_id)
        return witness

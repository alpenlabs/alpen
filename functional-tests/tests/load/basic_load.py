import flexitest

from envs import testenv
from utils import wait_for_proof_with_time_out
from utils.utils import wait_until_eth_block_exceeds


@flexitest.register
class BasicLoadGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("load_reth")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        reth = ctx.get_service("reth")
        prover_client_rpc = prover_client.create_rpc()
        rethrpc = reth.create_rpc()

        # Wait for a some blocks with transactions to be generated.
        block_num = wait_until_eth_block_exceeds(rethrpc, 25, timeout=30)
        self.info(f"Latest reth block={block_num}")
        self.test_checkpoint(50, block_num, prover_client_rpc)

    def test_checkpoint(self, l1_block, l2_block, prover_client_rpc):
        l1 = (1, l1_block)
        l2 = (1, l2_block)

        task_ids = prover_client_rpc.dev_strata_proveCheckpointRaw(0, l1, l2)

        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        assert wait_for_proof_with_time_out(prover_client_rpc, task_id, time_out=30)

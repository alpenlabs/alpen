import flexitest

from envs import testenv


@flexitest.register
class BasicLoadGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("load_reth")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        reth = ctx.get_service("reth")
        prover_client_rpc = prover_client.create_rpc()
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc)

        # Wait for a some blocks with transactions to be generated.
        block_num = reth_waiter.wait_until_eth_block_exceeds(25)
        self.info(f"Latest reth block={block_num}")
        self.test_checkpoint(50, block_num, prover_client_rpc)

    def test_checkpoint(self, l1_block, l2_block, prover_client_rpc):
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)
        l1 = (1, l1_block)
        l2 = (1, l2_block)

        task_ids = prover_client_rpc.dev_strata_proveCheckpointRaw(0, l1, l2)

        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        assert prover_waiter.wait_for_proof_completion(task_id)

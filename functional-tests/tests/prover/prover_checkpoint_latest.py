import flexitest

from envs import testenv


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        reth = ctx.get_service("reth")
        prover_client_rpc = prover_client.create_rpc()

        # Initialize prover waiter and wait for readiness
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30, interval=1)

        # Wait until some blocks are produced in EE, this is for creating dependent tasks in prover.
        reth_waiter = self.create_reth_waiter(reth.create_rpc())
        reth_waiter.wait_until_eth_block_at_least(20)

        prover_waiter.wait_until_prover_ready()

        # Test on with the latest checkpoint
        task_ids = prover_client_rpc.dev_strata_proveLatestCheckPoint()
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

import flexitest

from envs import testenv
from utils.wait import ProverWaiter


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()

        # Initialize prover waiter and wait for readiness
        prover_waiter = ProverWaiter(prover_client_rpc, self.logger, timeout=30, interval=1)
        prover_waiter.wait_for_prover_ready()

        # Test on with the latest checkpoint
        task_ids = prover_client_rpc.dev_strata_proveLatestCheckPoint()
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

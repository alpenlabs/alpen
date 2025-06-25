import flexitest

from envs import testenv
from utils import cl_slot_to_block_id

# Parameters defining the range of Execution Engine (EE) blocks to be proven.
# FIXME: cl_stf needs range to cover a full epoch so this test should be focused on epoch/checkpoint
# range instead of arbitrary range which will fail.
CL_PROVER_PARAMS = {
    "start_block": 1,
    "end_block": 1,
}


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        seq = ctx.get_service("sequencer")

        prover_client_rpc = prover_client.create_rpc()
        seqrpc = seq.create_rpc()
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)

        # Wait until the prover client reports readiness
        prover_waiter.wait_for_prover_ready()

        # Dispatch the prover task
        start_block_id = cl_slot_to_block_id(seqrpc, CL_PROVER_PARAMS["start_block"])
        start_block_commitment = {"slot": CL_PROVER_PARAMS["start_block"], "blkid": start_block_id}

        end_block_id = cl_slot_to_block_id(seqrpc, CL_PROVER_PARAMS["end_block"])
        end_block_commitment = {"slot": CL_PROVER_PARAMS["end_block"], "blkid": end_block_id}

        task_ids = prover_client_rpc.dev_strata_proveClBlocks(
            (start_block_commitment, end_block_commitment)
        )
        task_id = task_ids[0]

        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

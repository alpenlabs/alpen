import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import bytes_to_big_endian
from utils.wait import ProverWaiter


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        prover_client = ctx.get_service("prover_client")

        btcrpc: BitcoindClient = btc.create_rpc()
        prover_client_rpc = prover_client.create_rpc()

        # Wait until the prover client reports readiness
        prover_waiter = ProverWaiter(prover_client_rpc, self.logger, timeout=30, interval=2)
        prover_waiter.wait_for_prover_ready()

        # Dispatch the prover task
        block_height = 1
        blockhash = bytes_to_big_endian(btcrpc.proxy.getblockhash(block_height))
        block_commitment = {"height": block_height, "blkid": blockhash}

        task_ids = prover_client_rpc.dev_strata_proveBtcBlocks(
            (block_commitment, block_commitment), 0
        )
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id, timeout=30)
        assert is_proof_generation_completed

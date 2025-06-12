import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import bytes_to_big_endian, wait_for_proof_with_time_out, wait_until


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
        wait_until(
            lambda: prover_client_rpc.dev_strata_getReport() is not None,
            error_with="Prover did not start on time",
        )

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

        time_out = 30
        is_proof_generation_completed = wait_for_proof_with_time_out(
            prover_client_rpc, task_id, time_out=time_out
        )
        assert is_proof_generation_completed

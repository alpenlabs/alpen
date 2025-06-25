import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import bytes_to_big_endian

# Parameters defining therange of L1 blocks to be proven.
L1_PROVER_PARAMS = {
    "START_BLOCK_HEIGHT": 1,
    "END_BLOCK_HEIGHT": 3,
}


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        prover_client = ctx.get_service("prover_client")

        btcrpc: BitcoindClient = btc.create_rpc()
        prover_client_rpc = prover_client.create_rpc()
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)
        prover_waiter.wait_for_prover_ready()

        start_block_height = L1_PROVER_PARAMS["START_BLOCK_HEIGHT"]
        start_block_hash = bytes_to_big_endian(btcrpc.proxy.getblockhash(start_block_height))
        start_block = {"height": start_block_height, "blkid": start_block_hash}

        end_block_height = L1_PROVER_PARAMS["END_BLOCK_HEIGHT"]
        end_block_hash = bytes_to_big_endian(btcrpc.proxy.getblockhash(end_block_height))
        end_block = {"height": end_block_height, "blkid": end_block_hash}

        task_ids = prover_client_rpc.dev_strata_proveBtcBlocks((start_block, end_block), 0)
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"Using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

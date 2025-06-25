import flexitest
from web3 import Web3

from envs import testenv
from utils import el_slot_to_block_commitment
from utils.constants import PRECOMPILE_SCHNORR_ADDRESS
from utils.precompile import (
    get_schnorr_precompile_input,
    get_test_schnnor_secret_key,
    make_precompile_call,
)


@flexitest.register
class ProverClientTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc)

        web3: Web3 = reth.create_web3()

        # Wait for first EE block
        reth_waiter.wait_until_eth_block_at_least(1)

        secret_key = get_test_schnnor_secret_key()
        msg = "AlpenStrata"
        precompile_input = get_schnorr_precompile_input(secret_key, msg)
        txid, _data = make_precompile_call(web3, PRECOMPILE_SCHNORR_ADDRESS, precompile_input)

        txn = web3.eth.get_transaction(txid)
        block_number = txn.blockNumber

        # Parameters defining the range of Execution Engine (EE) blocks to be proven.
        ee_prover_params = {
            "start_block": block_number - 1,
            "end_block": block_number + 1,
        }

        # Wait for end EE block
        reth_waiter.wait_until_eth_block_at_least(ee_prover_params["end_block"])

        # Dispatch the prover task
        start_block = el_slot_to_block_commitment(rethrpc, ee_prover_params["start_block"])
        end_block = el_slot_to_block_commitment(rethrpc, ee_prover_params["end_block"])

        task_ids = prover_client_rpc.dev_strata_proveElBlocks((start_block, end_block))
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

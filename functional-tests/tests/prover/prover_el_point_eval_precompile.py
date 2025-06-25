import flexitest
from web3 import Web3

from mixins import BaseMixin
from utils import (
    el_slot_to_block_commitment,
    wait_for_proof_with_time_out,
    wait_until_with_value,
)
from utils.precompile import make_precompile_call


@flexitest.register
class ProverPointEvalPrecompileTest(BaseMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        reth_rpc = reth.create_rpc()

        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()

        web3: Web3 = reth.create_web3()
        web3.eth.default_account = web3.address

        precompile_address = web3.to_checksum_address("0x000000000000000000000000000000000000000a")
        precompile_input = "0xc00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"  # noqa: E501

        start_block = web3.eth.block_number
        end_block = web3.eth.block_number
        _txid, data = make_precompile_call(web3, precompile_address, precompile_input)
        # Assert that the precompile returns "0x0", confirming it is disabled.
        assert data == "0x", f"Point evaluation precompile failed: expected '0x0', got '{data}'."

        ee_prover_params = {"start_block": start_block, "end_block": end_block}

        # Wait until the end EE block is generated.
        wait_until_with_value(
            lambda: web3.eth.get_block("latest")["number"],
            lambda height: height >= ee_prover_params["end_block"],
            error_with="EE blocks not generated",
        )

        start_block = el_slot_to_block_commitment(reth_rpc, ee_prover_params["start_block"])
        end_block = el_slot_to_block_commitment(reth_rpc, ee_prover_params["end_block"])

        task_ids = prover_client_rpc.dev_strata_proveElBlocks((start_block, end_block))
        self.debug(f"Prover task IDs received: {task_ids}")

        if not task_ids:
            raise Exception("No task IDs received from prover_client_rpc")

        task_id = task_ids[0]
        self.debug(f"Using task ID: {task_id}")

        is_proof_generation_completed = wait_for_proof_with_time_out(
            prover_client_rpc, task_id, time_out=30
        )
        assert is_proof_generation_completed

import flexitest

from mixins import BaseMixin
from utils import (
    el_slot_to_block_commitment,
)
from utils.transaction import TransactionType
from utils.wait import ProverWaiter, RethWaiter

# Constants for native token transfer
NATIVE_TOKEN_TRANSFER_PARAMS = {
    "TRANSFER_AMOUNT": 1,
    "RECIPIENT": "0x5400000000000000000000000000000000000011",
}


@flexitest.register
class ProverClientTest(BaseMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()

        reth = ctx.get_service("reth")
        reth_rpc = reth.create_rpc()

        # Wait until at least one EE block is generated.
        reth_waiter = RethWaiter(reth_rpc, timeout=30, interval=1)
        reth_waiter.wait_until_eth_block_exceeds(0)

        transfer_amount = NATIVE_TOKEN_TRANSFER_PARAMS["TRANSFER_AMOUNT"]
        recipient = NATIVE_TOKEN_TRANSFER_PARAMS["RECIPIENT"]
        tx_receipt = self.txs.transfer(
            recipient, transfer_amount, TransactionType.LEGACY, wait=True
        )

        ee_prover_params = {
            "start_block": tx_receipt["blockNumber"] - 1,
            "end_block": tx_receipt["blockNumber"] + 1,
        }

        # Wait until the end EE block is generated.
        reth_waiter.wait_until_eth_block_exceeds(ee_prover_params["end_block"] - 1)

        start_block = el_slot_to_block_commitment(reth_rpc, ee_prover_params["start_block"])
        end_block = el_slot_to_block_commitment(reth_rpc, ee_prover_params["end_block"])

        task_ids = prover_client_rpc.dev_strata_proveElBlocks((start_block, end_block))
        self.debug(f"Prover task IDs received: {task_ids}")

        if not task_ids:
            raise Exception("No task IDs received from prover_client_rpc")

        task_id = task_ids[0]
        self.debug(f"Using task ID: {task_id}")

        prover_waiter = ProverWaiter(prover_client_rpc, self.logger, timeout=30, interval=2)
        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed

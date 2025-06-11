import flexitest

from mixins import BaseMixin
from utils import (
    el_slot_to_block_commitment,
    wait_for_proof_with_time_out,
    wait_until_with_value,
)


@flexitest.register
class ElSelfDestructToAddressContractTest(BaseMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        reth_rpc = reth.create_rpc()

        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()
        web3 = self.web3

        # Deploy the contracts
        # The setup is a simple two contacts relation:
        #  - Delegator is a receiver of the SELFDESTRUCT opcode.
        #  - Suicider's only purpose is to be constructed with delegator's address
        #       and suicide itself.
        # Later on, the Suicider is called to be destructed and reproduce the bug.
        DELEGATOR_ID = "delegator_contract"
        SUICIDER_ID = "suicider_contract"

        # STEP 1: deploy delegator and fetch its address
        delegator_address, _ = self.txs.deploy_contract("Counter.sol", "Counter", DELEGATOR_ID)

        # STEP 2: deploy suicider with delegator's address and fetch its address.
        self.txs.deploy_contract(
            "SelfDestructToAddress.sol", "SelfDestructToAddress", SUICIDER_ID, delegator_address
        )

        # STEP 3: Call the SelfDestructToAddress::suicide() contract function and invoke EL prove.
        suicide_call_tx_receipt = self.txs.call_contract(SUICIDER_ID, "suicide")

        # Prove the corresponding EE block
        ee_prover_params = {
            "start_block": suicide_call_tx_receipt["blockNumber"] - 1,
            "end_block": suicide_call_tx_receipt["blockNumber"] + 1,
        }

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

        assert wait_for_proof_with_time_out(prover_client_rpc, task_id, time_out=30)

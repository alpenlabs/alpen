import flexitest
from solcx import install_solc, set_solc_version
from web3 import Web3

from envs import testenv
from utils import (
    el_slot_to_block_commitment,
    wait_for_proof_with_time_out,
    wait_until_with_value,
)
from utils.transaction import SmartContracts


@flexitest.register
class ElSelfDestructToAddressContractTest(testenv.StrataTester):
    def __init__(self, ctx: flexitest.InitContext):
        install_solc(version="0.8.16")
        set_solc_version("0.8.16")
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        reth_rpc = reth.create_rpc()

        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()

        web3: Web3 = reth.create_web3()
        web3.eth.default_account = web3.address

        # Deploy the contracts
        # The setup is a simple two contacts relation:
        #  - Delegator is a receiver of the SELFDESTRUCT opcode.
        #  - Suicider's only purpose is to be constructed with delegator's address
        #       and suicide itself.
        # Later on, the Suicider is called to be destructed and reproduce the bug.

        # STEP 1: deploy delegator and fetch its address
        delegator_abi, delegator_bytecode = SmartContracts.compile_contract(
            "Counter.sol", "Counter"
        )
        delegator_contract = web3.eth.contract(abi=delegator_abi, bytecode=delegator_bytecode)
        delegator_tx_hash = delegator_contract.constructor().transact()
        tx_receipt_delegator = web3.eth.wait_for_transaction_receipt(delegator_tx_hash, timeout=30)
        delegator_address = tx_receipt_delegator["contractAddress"]

        # STEP 2: deploy suicider with delegator's address and fetch its address.
        suicider_abi, suicider_bytecode = SmartContracts.compile_contract(
            "SelfDestructToAddress.sol", "SelfDestructToAddress"
        )
        suicider_contract = web3.eth.contract(abi=suicider_abi, bytecode=suicider_bytecode)
        suicider_deploy_tx_hash = suicider_contract.constructor(delegator_address).transact()
        suicider_deploy_tx_receipt = web3.eth.wait_for_transaction_receipt(
            suicider_deploy_tx_hash, timeout=30
        )
        suicider_address = suicider_deploy_tx_receipt["contractAddress"]

        # STEP 3: Call the SelfDestructToAddress::suicide() contract function and invoke EL prove.
        contract_instance = web3.eth.contract(abi=suicider_abi, address=suicider_address)
        tx_hash = contract_instance.functions.suicide().transact()
        suicide_call_tx_receipt = web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

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

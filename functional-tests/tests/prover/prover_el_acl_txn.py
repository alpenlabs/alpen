import flexitest
from eth_typing import Address
from web3 import Web3

from envs import testenv
from utils import (
    el_slot_to_block_commitment,
    wait_until_with_value,
)


@flexitest.register
class ElCalldataTransactionProofTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("prover")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        reth_rpc = reth.create_rpc()

        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)

        web3: Web3 = reth.create_web3()
        web3.eth.default_account = web3.address

        # STEP1: deploy a simple smart contract.

        # The initcode of smart contract leads to the following deployed bytecode:
        # 0x6001600101600055
        # Opcode	Meaning
        # 0x60 01	PUSH1 0x01
        # 0x60 01	PUSH1 0x01
        # 0x01	    ADD
        # 0x60 00	PUSH1 0x00
        # 0x55	    SSTORE (at the slot 0)
        deploy_contract_tx = {
            "to": Address(b""),
            "data": "0x610008600081600b8239f36001600101600055",
            "gasPrice": 1000000000,
            "gas": 200000,
        }
        tx_hash = web3.eth.send_transaction(deploy_contract_tx)
        deploy_tx_receipt = web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

        # STEP 2: assemble the template for acl transaction (without the type and nonce).
        call_with_acl = {
            "to": deploy_tx_receipt["contractAddress"],
            "data": "0x",
            "accessList": [
                {
                    "address": "0x1111111111111111111111111111111111111111",
                    "storageKeys": [
                        "0x000000000000000000000000000000000000000000000000000000000000cdef"
                    ],
                }
            ],
            "gas": 200000,
            "value": 0,
        }

        # STEP 3: send two transactions (type 1 - EIP2930 and type 2 - EIP1559)
        call_with_acl_type_1 = call_with_acl.copy()
        call_with_acl_type_1.update({"type": 1, "gasPrice": 1000000000})

        call_with_acl_type_2 = call_with_acl.copy()
        call_with_acl_type_2.update(
            {
                "type": 2,
                "maxFeePerGas": 1000000000,
                "maxPriorityFeePerGas": 1000000000,
            }
        )

        tx_hash = web3.eth.send_transaction(call_with_acl_type_1)
        web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

        tx_hash = web3.eth.send_transaction(call_with_acl_type_2)
        last_tx = web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

        # STEP 4: Prove the corresponding EE block
        ee_prover_params = {
            "start_block": deploy_tx_receipt["blockNumber"] - 1,
            "end_block": last_tx["blockNumber"] + 1,
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

        assert prover_waiter.wait_for_proof_completion(task_id)

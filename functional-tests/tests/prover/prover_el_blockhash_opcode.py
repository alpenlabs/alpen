import flexitest
from web3 import Web3

from mixins import BaseMixin
from utils import (
    el_slot_to_block_commitment,
    wait_until_with_value,
)

# An identifier of BlockhashOpCode contract to work with.
BLOCKHASH_CTR_ID = "blockhash_contract"


@flexitest.register
class ElBlockhashOpcodeTest(BaseMixin):
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

        # Deploy and call the contract

        self.txs.deploy_contract("BlockhashOpCode.sol", "BlockhashOpCode", BLOCKHASH_CTR_ID)
        tx_receipt = self.txs.call_contract(BLOCKHASH_CTR_ID, "updateBlockHash")

        # Prove the corresponding EE block
        ee_prover_params = {
            "start_block": tx_receipt["blockNumber"] - 1,
            "end_block": tx_receipt["blockNumber"] + 1,
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

        lastBlockHash = (
            self.txs.get_contract_instance(BLOCKHASH_CTR_ID).functions.lastBlockHash().call()
        )
        self.debug(f"lastBlockHash: {type(lastBlockHash)} {lastBlockHash.hex()}")

        assert prover_waiter.wait_for_proof_completion(task_id)

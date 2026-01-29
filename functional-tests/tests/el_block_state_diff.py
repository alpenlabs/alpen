import logging

import flexitest
from web3 import Web3

from envs import testenv
from utils import compile_solidity


@flexitest.register
class ElBlockStateDiffDataGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("state_diffs")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()
        reth_waiter = self.create_reth_waiter(rethrpc)

        web3: Web3 = reth.create_web3()
        web3.eth.default_account = web3.address

        # Deploy the contract
        abi, bytecode = get_contract()
        contract = web3.eth.contract(abi=abi, bytecode=bytecode)
        tx_hash = contract.constructor().transact()
        tx_receipt = web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

        # Get the block hash where contract was deployed
        assert tx_receipt["status"] == 1
        blocknum = tx_receipt.blockNumber
        blockhash = rethrpc.eth_getBlockByNumber(hex(blocknum), False)["hash"]

        # wait for witness data generation
        state_diff_data = reth_waiter.wait_until_state_diff_at_blockhash(blockhash, timeout=2)
        logging.info(state_diff_data)

        # Get the actual state root from the block
        block = rethrpc.eth_getBlockByNumber(hex(blocknum), False)
        actual_state_root = block["stateRoot"]
        logging.info(f"Actual state root from block: {actual_state_root}")

        # Get state root reconstructed from state diffs
        reconstructed_state_root = rethrpc.strataee_getStateRootByDiffs(blocknum)
        logging.info(f"Reconstructed state root from diffs: {reconstructed_state_root}")

        # Validate they match
        assert actual_state_root == reconstructed_state_root, (
            f"State root mismatch! Actual: {actual_state_root}, "
            f"Reconstructed: {reconstructed_state_root}"
        )


def get_contract() -> tuple[list, str]:
    return compile_solidity(
        """
        pragma solidity ^0.8.0;

        contract Greeter {
            string public greeting;

            constructor() {
                greeting = 'Hello';
            }
        }
        """
    )

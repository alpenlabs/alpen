import flexitest
from web3 import Web3

from envs import testenv
from utils.eth import make_native_token_transfer

NATIVE_TOKEN_TRANSFER_PARAMS = {
    "DEST_ADDRESS": "0x0000000000000000000000000000000000000001",
    "BASEFEE_ADDRESS": "5400000000000000000000000000000000000010",
    "BENEFICIARY_ADDRESS": "5400000000000000000000000000000000000011",
    "TRANSFER_AMOUNT": Web3.to_wei(1, "ether"),
}


@flexitest.register
class ElBalanceTransferTest(testenv.StrataTester):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        source = web3.address
        dest = web3.to_checksum_address(NATIVE_TOKEN_TRANSFER_PARAMS["DEST_ADDRESS"])

        self.debug(f"{web3.is_connected()}")
        original_block_no = web3.eth.block_number
        dest_original_balance = web3.eth.get_balance(dest)
        source_original_balance = web3.eth.get_balance(source)

        self.debug(f"{original_block_no}, {dest_original_balance}")

        transfer_amount = NATIVE_TOKEN_TRANSFER_PARAMS["TRANSFER_AMOUNT"]
        _tx_receipt = make_native_token_transfer(web3, transfer_amount, dest)

        self.debug(f"Got txn receipt: {_tx_receipt}")

        final_block_no = web3.eth.block_number
        dest_final_balance = web3.eth.get_balance(dest)
        source_final_balance = web3.eth.get_balance(source)

        self.debug(f"{final_block_no}, {dest_final_balance}")

        assert original_block_no < final_block_no, (
            f"Expected final block number ({final_block_no}) "
            f"to be greater than original ({original_block_no})"
        )
        assert dest_original_balance + transfer_amount == dest_final_balance, (
            f"Expected dest_final_balance ({dest_final_balance}) to equal "
            f"dest_original_balance ({dest_original_balance}) + transfer_amount ({transfer_amount})"
        )
        assert dest_final_balance == transfer_amount, (
            f"Expected dest_final_balance ({dest_final_balance}) to equal "
            f"transfer_amount ({transfer_amount})"
        )
        assert source_original_balance > source_final_balance + transfer_amount, (
            f"Expected source_original_balance ({source_original_balance}) to be greater than "
            f"source_final_balance ({source_final_balance}) + transfer_amount ({transfer_amount})"
        )

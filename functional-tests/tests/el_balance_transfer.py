import flexitest
from web3 import Web3

from mixins import BaseMixin
from utils.transaction import TransactionType

NATIVE_TOKEN_TRANSFER_PARAMS = {
    "DEST_ADDRESS": "0x0000000000000000000000000000000000000001",
    "BASEFEE_ADDRESS": "5400000000000000000000000000000000000010",
    "BENEFICIARY_ADDRESS": "5400000000000000000000000000000000000011",
    "TRANSFER_AMOUNT": 1,  # 1 ETH
}


@flexitest.register
class ElBalanceTransferTest(BaseMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, _ctx: flexitest.RunContext):
        web3: Web3 = self.w3

        source = web3.address
        dest = web3.to_checksum_address(NATIVE_TOKEN_TRANSFER_PARAMS["DEST_ADDRESS"])
        basefee_address = web3.to_checksum_address(NATIVE_TOKEN_TRANSFER_PARAMS["BASEFEE_ADDRESS"])
        beneficiary_address = web3.to_checksum_address(
            NATIVE_TOKEN_TRANSFER_PARAMS["BENEFICIARY_ADDRESS"]
        )

        self.debug(f"{web3.is_connected()}")
        original_block_no = web3.eth.block_number
        dest_original_balance = web3.eth.get_balance(dest)
        source_original_balance = web3.eth.get_balance(source)
        basefee_original_balance = web3.eth.get_balance(basefee_address)
        beneficiary_original_balance = web3.eth.get_balance(beneficiary_address)

        self.debug(f"{original_block_no}, {dest_original_balance}")

        transfer_amount = NATIVE_TOKEN_TRANSFER_PARAMS["TRANSFER_AMOUNT"]
        self.txs.transfer(dest, transfer_amount, TransactionType.LEGACY, wait=True)

        # Original value is in eth (conversion happens under the hood in `txs.transfer`,
        # so convert to wei to perform exact checks on balances.
        transfer_amount = Web3.to_wei(transfer_amount, "ether")

        final_block_no = web3.eth.block_number
        dest_final_balance = web3.eth.get_balance(dest)
        source_final_balance = web3.eth.get_balance(source)
        basefee_final_balance = web3.eth.get_balance(basefee_address)
        beneficiary_final_balance = web3.eth.get_balance(beneficiary_address)

        self.debug(f"{final_block_no}, {dest_final_balance}")

        assert original_block_no < final_block_no
        assert dest_original_balance + transfer_amount == dest_final_balance

        basefee_balance_change = basefee_final_balance - basefee_original_balance
        self.debug(
            f"basefee balance change: {basefee_balance_change} "
            f"before: {basefee_original_balance} "
            f"after: {basefee_final_balance}"
        )
        assert basefee_balance_change > 0
        beneficiary_balance_change = beneficiary_final_balance - beneficiary_original_balance
        assert beneficiary_balance_change > 0
        source_balance_change = source_final_balance - source_original_balance
        assert (
            source_balance_change
            + basefee_balance_change
            + beneficiary_balance_change
            + transfer_amount
            == 0
        ), "total balance change is not balanced"

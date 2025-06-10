import time

import flexitest
from strata_utils import (
    deposit_request_transaction,
    is_valid_bosd,
)

from envs.rollup_params_cfg import RollupConfig
from utils import *
from utils.constants import PRECOMPILE_BRIDGEOUT_ADDRESS

from . import BaseMixin

# Local constants
# Ethereum Private Key
# NOTE: don't use this private key in production
ETH_PRIVATE_KEY = "0x0000000000000000000000000000000000000000000000000000000000000001"


class BridgeMixin(BaseMixin):
    """
    Mixin for bridge specific functionality in the tests.
    Provides methods for setting up service, making DRT, withdraw transaction
    """

    def premain(self, ctx: flexitest.RunContext):
        super().premain(ctx)

        self.bridge_eth_account = self.web3.eth.account.from_key(ETH_PRIVATE_KEY)

    def deposit(self, ctx: flexitest.RunContext, el_address, bridge_pk) -> str:
        """
        Make DRT deposit to the EL address. Wait until the deposit is reflected on L2.

        Returns the transaction id of the DRT on the bitcoin regtest.
        """
        cfg: RollupConfig = ctx.env.rollup_cfg()
        # D BTC
        deposit_amount = cfg.deposit_amount

        # bridge pubkey
        self.info(f"Bridge pubkey: {bridge_pk}")

        # check balance before deposit
        initial_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        self.info(f"Strata Balance right before deposit calls: {initial_balance}")

        tx_id = self.make_drt(el_address, bridge_pk)

        # Wait until the deposit is seen on L2
        expected_balance = initial_balance + deposit_amount * SATS_TO_WEI
        wait_until(
            lambda: int(self.rethrpc.eth_getBalance(el_address), 16) == expected_balance,
            error_with="Strata balance after deposit is not as expected",
        )

        return tx_id

    def withdraw(
        self,
        ctx: flexitest.RunContext,
        el_address: str,
        destination: str,
    ):
        """
        Perform a withdrawal from the L2 to the given BTC withdraw destination.
        Returns (l2_tx_hash, tx_receipt, total_gas_used).

        NOTE: The withdrawal destination is a Bitcoin Output Script Descriptor (BOSD).
        """
        cfg: RollupConfig = ctx.env.rollup_cfg()
        # D BTC
        deposit_amount = cfg.deposit_amount
        # Build the BOSD descriptor from the withdraw address
        # Assert is a valid BOSD
        assert is_valid_bosd(destination), "Invalid BOSD"
        self.info(f"Withdrawal Destination: {destination}")

        # Estimate gas
        estimated_withdraw_gas = self.__estimate_withdraw_gas(
            deposit_amount, el_address, destination
        )
        self.info(f"Estimated withdraw gas: {estimated_withdraw_gas}")

        l2_tx_hash = self.__make_withdraw(
            deposit_amount, el_address, destination, estimated_withdraw_gas
        ).hex()
        self.info(f"Sent withdrawal transaction with hash: {l2_tx_hash}")

        # Wait for transaction receipt
        tx_receipt = wait_until_with_value(
            lambda: self.web3.eth.get_transaction_receipt(l2_tx_hash),
            predicate=lambda v: v is not None,
        )
        self.info(f"Transaction receipt: {tx_receipt}")

        total_gas_used = tx_receipt["gasUsed"] * tx_receipt["effectiveGasPrice"]
        self.info(f"Total gas used: {total_gas_used}")

        # Ensure the leftover in the EL address is what's expected (deposit minus gas)
        balance_post_withdraw = int(self.rethrpc.eth_getBalance(el_address), 16)
        difference = deposit_amount * SATS_TO_WEI - total_gas_used
        self.info(f"Strata Balance after withdrawal: {balance_post_withdraw}")
        self.info(f"Strata Balance difference: {difference}")
        assert difference == balance_post_withdraw, "balance difference is not expected"

        return l2_tx_hash, tx_receipt, total_gas_used

    def __make_withdraw(
        self,
        deposit_amount,
        el_address,
        destination,
        gas,
    ):
        """
        Withdrawal Request Transaction in Strata's EVM.

        NOTE: The withdrawal destination is a Bitcoin Output Script Descriptor (BOSD).
        """
        assert is_valid_bosd(destination), "Invalid BOSD"

        data_bytes = bytes.fromhex(destination)

        transaction = {
            "from": el_address,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": deposit_amount * SATS_TO_WEI,
            "gas": gas,
            "data": data_bytes,
        }
        l2_tx_hash = self.web3.eth.send_transaction(transaction)
        return l2_tx_hash

    def __estimate_withdraw_gas(self, deposit_amount, el_address, destination):
        """
        Estimate the gas for the withdrawal transaction.

        NOTE: The withdrawal destination is a Bitcoin Output Script Descriptor (BOSD).
        """

        assert is_valid_bosd(destination), "Invalid BOSD"

        data_bytes = bytes.fromhex(destination)

        transaction = {
            "from": el_address,
            "to": PRECOMPILE_BRIDGEOUT_ADDRESS,
            "value": deposit_amount * SATS_TO_WEI,
            "data": data_bytes,
        }
        return self.web3.eth.estimate_gas(transaction)

    def make_drt(self, el_address, musig_bridge_pk):
        """
        Deposit Request Transaction

        Returns the transaction id of the DRT on the bitcoin regtest.
        """
        # Get relevant data
        btc_url = self.btcrpc.base_url
        btc_user = self.btc.get_prop("rpc_user")
        btc_password = self.btc.get_prop("rpc_password")
        seq_addr = self.seq.get_prop("address")

        # Create the deposit request transaction
        tx = bytes(
            deposit_request_transaction(
                el_address, musig_bridge_pk, btc_url, btc_user, btc_password
            )
        ).hex()

        # Send the transaction to the Bitcoin network
        drt_tx_id: str = self.btcrpc.proxy.sendrawtransaction(tx)

        time.sleep(1)

        # time to mature DRT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        time.sleep(3)

        # time to mature DT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        time.sleep(3)
        return drt_tx_id

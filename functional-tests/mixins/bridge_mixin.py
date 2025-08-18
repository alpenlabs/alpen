import flexitest
import threading
from strata_utils import (
    deposit_request_transaction,
    is_valid_bosd,
    create_deposit_transaction,
    create_withdrawal_fulfillment,
)

from envs.rollup_params_cfg import RollupConfig
from utils import *
from utils.utils import wait_until, wait_until_with_value
from utils.constants import PRECOMPILE_BRIDGEOUT_ADDRESS
from utils.wait import StrataWaiter

from . import BaseMixin

# Local constants
# Ethereum Private Key
# NOTE: don't use this private key in production
ETH_PRIVATE_KEY = "0x0000000000000000000000000000000000000000000000000000002000000011"


class BridgeMixin(BaseMixin):
    """
    Mixin for bridge specific functionality in the tests.
    Provides methods for setting up service, making DRT, withdraw transaction
    """

    def premain(self, ctx: flexitest.RunContext):
        super().premain(ctx)

        self.bridge_eth_account = self.w3.eth.account.from_key(ETH_PRIVATE_KEY)
        self.web3 = self.w3

        # Bridge manager state
        self.next_deposit_id = 0
        self.deposit_txids = {}  # deposit_id -> txid mapping

        # Synchronization locks
        self.deposit_lock = threading.Lock()
        self.withdrawal_lock = threading.Lock()

    def deposit(self, ctx: flexitest.RunContext, el_address, priv_keys) -> tuple[str, int, str]:
        """
        Make DRT deposit and managed DT with block generation and waiting.
        Handles the complete deposit flow including synchronization and balance verification.

        Returns (drt_tx_id, deposit_id, dt_tx_id)
        """
        with self.deposit_lock:
            cfg: RollupConfig = ctx.env.rollup_cfg()
            deposit_amount = cfg.deposit_amount

            # Get initial state
            initial_deposits = len(self.seqrpc.strata_getCurrentDeposits())
            initial_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
            self.info(f"Initial deposit count: {initial_deposits}")
            self.info(f"Initial EL balance: {initial_balance}")

            # Make DRT (deposit request transaction)
            drt_tx_id = self.make_drt(el_address, priv_keys)
            self.info(f"Deposit Request Transaction ID: {drt_tx_id}")
            print(f"Deposit Request Transaction ID: {drt_tx_id}")

            # Create managed DT (deposit transaction) with auto-incremented ID
            deposit_id, dt_tx_id = self.managed_deposit(ctx, el_address, priv_keys)

            # Generate blocks to mature the deposit transaction
            seq_addr = self.seq.get_prop("address")
            self.btcrpc.proxy.generatetoaddress(6, seq_addr)

            # Wait for exactly one new deposit to appear
            expected_deposit_count = initial_deposits + 1
            wait_until(
                lambda: len(self.seqrpc.strata_getCurrentDeposits()) >= expected_deposit_count,
                error_with=f"Timeout waiting for deposit to appear (expected {expected_deposit_count})",
                timeout=30,
                step=1
            )

            # Verify balance increased by deposit amount
            expected_balance = initial_balance + (deposit_amount * SATS_TO_WEI)
            wait_until(
                lambda: int(self.rethrpc.eth_getBalance(el_address), 16) >= expected_balance,
                error_with=f"Timeout waiting for EL balance to reflect deposit (expected >= {expected_balance})",
                timeout=30,
                step=1
            )

            final_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
            balance_increase = final_balance - initial_balance
            self.info(f"Deposit {deposit_id} confirmed: DT txid={dt_tx_id}, balance increased by {balance_increase}")
            print(f"Deposit {deposit_id}: Balance {initial_balance} -> {final_balance} (+{balance_increase})")

            return drt_tx_id, deposit_id, dt_tx_id

    def withdraw(
        self,
        ctx: flexitest.RunContext,
        el_address: str,
        destination: str,
    ):
        """
        Perform withdrawal from L2 to BTC destination with block generation and waiting.
        Handles the complete withdrawal flow including synchronization.

        Returns (l2_tx_hash, tx_receipt, total_gas_used)
        """
        with self.withdrawal_lock:
            deposit_amount = 1000000000
            assert is_valid_bosd(destination), "Invalid BOSD"
            self.info(f"Withdrawal Destination: {destination}")

            # Get initial withdrawal intent count
            initial_intents = len(self.seqrpc.strata_getWithdrawalIntent())
            self.info(f"Initial withdrawal intent count: {initial_intents}")

            # Estimate gas and make withdrawal
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
            # Generate blocks to process withdrawal and capture L1 height range
            seq_addr = self.seq.get_prop("address")
            withdrawal_height_start = self.btcrpc.proxy.getblockcount()
            self.btcrpc.proxy.generatetoaddress(10, seq_addr)
            withdrawal_height_end = self.btcrpc.proxy.getblockcount()

            self.info(f"Withdrawal L2 transaction in L1 height range: {withdrawal_height_start + 1} - {withdrawal_height_end}")

            # Wait for L1 blocks to be processed by the sequencer
            strata_waiter = StrataWaiter(self.seqrpc, self.logger, timeout=60, interval=1)
            strata_waiter.wait_until_l1_height_at(withdrawal_height_end)

            # Wait for checkpoint that covers the withdrawal L2 transaction
            initial_checkpoint_idx = self.seqrpc.strata_getLatestCheckpointIndex() or 0
            self.info(f"Initial checkpoint index: {initial_checkpoint_idx}")
            self.info(f"Waiting for checkpoint that includes L1 height range {withdrawal_height_start + 1}-{withdrawal_height_end}")

            def check_checkpoint_covers_withdrawal():
                latest_checkpoint_idx = self.seqrpc.strata_getLatestCheckpointIndex()
                if latest_checkpoint_idx is None or latest_checkpoint_idx <= initial_checkpoint_idx:
                    self.info(f"No new checkpoint yet (current: {latest_checkpoint_idx})")
                    return False

                # Check if the latest checkpoint covers our withdrawal height range
                checkpoint_info = self.seqrpc.strata_getCheckpointInfo(latest_checkpoint_idx)
                if checkpoint_info is None:
                    self.info(f"Checkpoint {latest_checkpoint_idx} info not available yet")
                    return False

                l1_start = checkpoint_info['l1_range'][0]['height']
                l1_end = checkpoint_info['l1_range'][1]['height']
                covers_range = l1_start <= withdrawal_height_start and l1_end >= withdrawal_height_end

                self.info(f"Checkpoint {latest_checkpoint_idx}: L1 range [{l1_start}, {l1_end}], covers withdrawal [{withdrawal_height_start + 1}, {withdrawal_height_end}]: {covers_range}")

                return covers_range

            # Wait for checkpoint that covers our withdrawal transaction
            wait_until(
                check_checkpoint_covers_withdrawal,
                error_with=f"Timeout waiting for checkpoint to cover withdrawal transaction at L1 heights {withdrawal_height_start + 1}-{withdrawal_height_end}",
                timeout=120,
                step=3
            )

            # Now wait for withdrawal intent to appear
            expected_intent_count = initial_intents + 1
            self.info(f"Checkpoint created, now waiting for withdrawal intent to appear (expected {expected_intent_count})")
            wait_until(
                lambda: len(self.seqrpc.strata_getWithdrawalIntent()) >= expected_intent_count,
                error_with=f"Timeout waiting for withdrawal intent after checkpoint creation (expected {expected_intent_count})",
                timeout=60,
                step=2
            )

            total_gas_used = tx_receipt["gasUsed"] * tx_receipt["effectiveGasPrice"]
            self.info(f"Total gas used: {total_gas_used}")

            balance_post_withdraw = int(self.rethrpc.eth_getBalance(el_address), 16)
            self.info(f"Strata Balance after withdrawal: {balance_post_withdraw}")

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
            # "gas": gas,
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

    def make_drt(self, el_address, priv_keys):
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
                el_address, priv_keys, btc_url, btc_user, btc_password
            )
        ).hex()

        # Send the transaction to the Bitcoin network
        drt_tx_id: str = self.btcrpc.proxy.sendrawtransaction(tx)
        current_height = self.btcrpc.proxy.getblockcount()

        # time to mature DRT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        # Wait for DRT maturation
        strata_waiter = StrataWaiter(self.seqrpc, self.logger, timeout=30, interval=1)
        strata_waiter.wait_until_l1_height_at(current_height+6)

        # time to mature DT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        # Wait for DT maturation
        strata_waiter.wait_until_l1_height_at(current_height + 12)
        return drt_tx_id

    def managed_deposit(self, ctx: flexitest.RunContext, el_address: str, priv_keys) -> tuple[int, str]:
        """
        Bridge manager deposit: creates deposit transaction with auto-incremented ID
        Returns (deposit_id, tx_id)
        """
        deposit_id = self.next_deposit_id
        self.next_deposit_id += 1

        # Create deposit transaction with managed ID
        tx = bytes(create_deposit_transaction(deposit_id, priv_keys)).hex()

        # Send transaction to Bitcoin network
        dt_tx_id = self.btcrpc.proxy.sendrawtransaction(tx)

        # Store txid for this deposit
        self.deposit_txids[deposit_id] = dt_tx_id

        self.info(f"Created managed deposit {deposit_id} with txid: {dt_tx_id}")
        print(f"Managed Deposit {deposit_id} Transaction ID: {dt_tx_id}")

        return deposit_id, dt_tx_id

    def get_deposit_txid(self, deposit_id: int) -> str:
        """Get stored txid for a deposit ID"""
        if deposit_id not in self.deposit_txids:
            raise ValueError(f"No txid stored for deposit_id {deposit_id}")
        return self.deposit_txids[deposit_id]

    def get_all_deposits(self) -> dict[int, str]:
        """Get all stored deposit_id -> txid mappings"""
        return self.deposit_txids.copy()

    def fulfill_withdrawal_intents(self, ctx: flexitest.RunContext) -> list[str]:
        """
        Process withdrawal intents by creating Bitcoin withdrawal fulfillment transactions.
        Waits for withdrawal intents to be processed and removed from the list.
        Returns list of withdrawal fulfillment txids
        """
        with self.withdrawal_lock:
            btc_url = self.btcrpc.base_url
            btc_user = self.btc.get_prop("rpc_user")
            btc_password = self.btc.get_prop("rpc_password")

            # Get initial withdrawal intents from sequencer
            initial_withdrawal_intents = self.seqrpc.strata_getWithdrawalIntent()
            initial_intent_count = len(initial_withdrawal_intents)
            self.info(f"Found {initial_intent_count} withdrawal intents to fulfill")

            if initial_intent_count == 0:
                self.info("No withdrawal intents to fulfill")
                return []

            fulfillment_txids = []

            for intent in initial_withdrawal_intents:
                try:
                    # Create withdrawal fulfillment transaction on Bitcoin
                    tx = create_withdrawal_fulfillment(
                        intent['destination'],
                        intent['amt'],
                        intent['operator_idx'],
                        intent['deposit_idx'],
                        intent['deposit_txid'],
                        btc_url,
                        btc_user,
                        btc_password
                    )

                    tx_hex = bytes(tx).hex()
                    wft_tx_id = self.btcrpc.proxy.sendrawtransaction(tx_hex)
                    fulfillment_txids.append(wft_tx_id)

                    self.info(f"Created withdrawal fulfillment txid: {wft_tx_id}")
                    print(f"Withdrawal fulfillment txid: {wft_tx_id}")

                except Exception as e:
                    self.error(f"Failed to create withdrawal fulfillment for intent {intent}: {e}")
                    raise

            # Generate blocks to mature fulfillment transactions
            seq_addr = self.seq.get_prop("address")
            self.btcrpc.proxy.generatetoaddress(6, seq_addr)

            # Wait for withdrawal intents to be processed and removed
            expected_final_count = initial_intent_count - len(fulfillment_txids)
            self.info(f"Waiting for withdrawal intents to be processed (expecting {expected_final_count} remaining)")

            wait_until(
                lambda: len(self.seqrpc.strata_getWithdrawalIntent()) <= expected_final_count,
                error_with=f"Timeout waiting for withdrawal intents to be processed (expected <= {expected_final_count})",
                timeout=60,
                step=2
            )

            final_intent_count = len(self.seqrpc.strata_getWithdrawalIntent())
            self.info(f"Withdrawal fulfillment complete: {initial_intent_count} -> {final_intent_count} intents")

            return fulfillment_txids

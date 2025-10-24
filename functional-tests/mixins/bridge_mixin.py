from typing import Any

import flexitest
from web3.middleware.signing import SignAndSendRawMiddlewareBuilder

from envs.rollup_params_cfg import RollupConfig
from factory.test_cli import create_deposit_transaction, create_withdrawal_fulfillment
from utils.utils import SATS_TO_WEI, wait_until, wait_until_with_value
from utils.wait import StrataWaiter

from . import BaseMixin

# Local constants
# Ethereum Private Key
# NOTE: don't use this private key in production
ETH_PRIVATE_KEY = "0x0000000000000000000000000000000000000000000000000000000000000001"


class BridgeMixin(BaseMixin):
    """
    Bridge operations mixin for functional tests.
    Handles deposits, withdrawals, and transaction fulfillment.
    """

    def premain(self, ctx: flexitest.RunContext):
        """Initialize bridge-specific test setup including Web3 middleware."""
        super().premain(ctx)

        self.bridge_eth_account = self.w3.eth.account.from_key(ETH_PRIVATE_KEY)
        self.w3.address = self.bridge_eth_account.address
        self.w3.middleware_onion.add(SignAndSendRawMiddlewareBuilder.build(self.bridge_eth_account))
        self.web3 = self.w3

    def deposit(
        self, ctx: flexitest.RunContext, el_address: str, priv_keys: list[Any]
    ) -> tuple[str, str]:
        """
        Make DRT deposit and managed DT with block generation and waiting.
        Handles the complete deposit flow including synchronization and balance verification.

        Returns (drt_tx_id, dt_tx_id)
        """
        cfg: RollupConfig = ctx.env.rollup_cfg()
        deposit_amount = cfg.deposit_amount

        # Get initial state
        initial_deposits = len(self.seqrpc.strata_getCurrentDeposits())
        initial_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        self.info(f"Initial deposit count: {initial_deposits}")
        self.info(f"Initial EL balance: {initial_balance}")

        # Make DRT (deposit request transaction)
        drt_tx_id, raw_drt_bytes = self.make_drt(el_address)
        self.info(f"Deposit Request Transaction ID: {drt_tx_id}")

        # Create managed DT (deposit transaction) with auto-incremented ID
        dt_tx_id = self.managed_deposit(raw_drt_bytes, priv_keys)

        # Generate blocks to mature the deposit transaction
        seq_addr = self.seq.get_prop("address")
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)

        # Wait for exactly one new deposit to appear
        expected_deposit_count = initial_deposits + 1
        wait_until(
            lambda: len(self.seqrpc.strata_getCurrentDeposits()) >= expected_deposit_count,
            error_with=(
                f"Timeout waiting for deposit to appear (expected {expected_deposit_count})"
            ),
            timeout=30,
            step=1,
        )

        # Verify balance increased by deposit amount
        expected_balance = initial_balance + (deposit_amount * SATS_TO_WEI)
        wait_until(
            lambda: int(self.rethrpc.eth_getBalance(el_address), 16) >= expected_balance,
            error_with=(
                f"Timeout waiting for EL balance to reflect deposit "
                f"(expected >= {expected_balance})"
            ),
            timeout=30,
            step=1,
        )

        final_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        balance_increase = final_balance - initial_balance
        self.info(f"Deposit confirmed: DT txid={dt_tx_id}, balance increased by {balance_increase}")

        return drt_tx_id, dt_tx_id

    def withdraw(self, el_address: str) -> tuple[str, Any, int]:
        """
        Perform withdrawal from L2 to BTC destination with block generation and waiting.
        Handles the complete withdrawal flow including synchronization.

        Returns (l2_tx_hash, tx_receipt, total_gas_used)
        """

        # Get initial withdrawal intent count
        initial_intents = len(self.seqrpc.strata_getCurWithdrawalAssignments())
        self.info(f"Initial withdrawal intent count: {initial_intents}")

        # Make withdrawal transaction
        l2_tx_hash = self.alpen_cli.withdraw()
        self.info(f"Sent withdrawal transaction with hash: {l2_tx_hash}")

        # Wait for transaction receipt
        tx_receipt = wait_until_with_value(
            lambda: self.web3.eth.get_transaction_receipt(l2_tx_hash),
            predicate=lambda v: v is not None,
        )
        # Generate blocks to process withdrawal and capture L1 height range
        withdrawal_height_start = self.btcrpc.proxy.getblockcount()

        self.info(f"Withdrawal L2 transaction in L1 height range: {withdrawal_height_start + 1}")

        # Wait for checkpoint that covers the withdrawal L2 transaction
        initial_checkpoint_idx = self.seqrpc.strata_getLatestCheckpointIndex() or 0
        self.info(f"Initial checkpoint index: {initial_checkpoint_idx}")
        self.info(f"Waiting for checkpoint that includes L1 height: {withdrawal_height_start}")

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

            l1_start = checkpoint_info["l1_range"][0]["height"]
            l1_end = checkpoint_info["l1_range"][1]["height"]
            covers_range = l1_end >= withdrawal_height_start

            self.info(
                f"Checkpoint {latest_checkpoint_idx}: L1 range [{l1_start}, {l1_end}], "
                f"covers withdrawal {withdrawal_height_start}: "
            )

            return covers_range

        # Wait for checkpoint that covers our withdrawal transaction
        wait_until(
            check_checkpoint_covers_withdrawal,
            error_with=(
                f"Timeout waiting for checkpoint to cover withdrawal transaction at "
                f"L1 heights {withdrawal_height_start}"
            ),
            timeout=120,
            step=3,
        )

        # Now wait for withdrawal intent to appear
        expected_intent_count = initial_intents + 1
        self.info(
            f"Checkpoint created, now waiting for withdrawal intent to appear "
            f"(expected {expected_intent_count})"
        )
        wait_until(
            lambda: len(self.seqrpc.strata_getCurWithdrawalAssignments()) >= expected_intent_count,
            error_with=(
                f"Timeout waiting for withdrawal intent after checkpoint creation "
                f"(expected {expected_intent_count})"
            ),
            timeout=60,
            step=2,
        )

        total_gas_used = tx_receipt["gasUsed"] * tx_receipt["effectiveGasPrice"]
        self.info(f"Total gas used: {total_gas_used}")

        balance_post_withdraw = int(self.rethrpc.eth_getBalance(el_address), 16)
        self.info(f"Strata Balance after withdrawal: {balance_post_withdraw}")

        return l2_tx_hash, tx_receipt, total_gas_used

    def make_drt(self, el_address=None) -> tuple[str, str]:
        """
        Creates and matures a Deposit Request Transaction (DRT).

        Returns:
            tuple[str, str]: (transaction_id, raw_transaction_hex)
        """
        # Get relevant data
        seq_addr = self.seq.get_prop("address")

        addr = self.alpen_cli.l1_address()
        # Fund bridge address and confirm with one block
        self.btcrpc.proxy.sendtoaddress(addr, 10.01)
        self.btcrpc.proxy.generatetoaddress(1, seq_addr)
        # Create and send deposit request transaction
        drt_tx_id = self.alpen_cli.deposit(el_address)
        current_height = self.btcrpc.proxy.getblockcount()
        # time to mature DRT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        # Wait for DRT maturation
        strata_waiter = StrataWaiter(self.seqrpc, self.logger, timeout=30, interval=1)
        strata_waiter.wait_until_l1_height_at(current_height + 6)
        drt_raw_tx = self.btcrpc.proxy.getrawtransaction(drt_tx_id)

        return drt_tx_id, drt_raw_tx

    def managed_deposit(self, raw_drt_tx: str, priv_keys: list[Any]) -> str:
        """
        Creates deposit transaction (DT) from DRT with auto-incremented ID.

        Args:
            raw_drt_tx: Raw DRT transaction hex
            priv_keys: Operator private keys for multi-sig

        Returns:
            str: Deposit transaction ID
        """
        seq_addr = self.seq.get_prop("address")

        # index of deposit transaction, this works
        # because deposit id is monotonically increasing id
        index = len(self.seqrpc.strata_getCurrentDeposits())
        raw_drt_tx = bytes.fromhex(raw_drt_tx)
        # Create deposit transaction with managed ID
        tx = bytes(create_deposit_transaction(raw_drt_tx, priv_keys, index)).hex()
        # Send transaction to Bitcoin network
        dt_tx_id = self.btcrpc.proxy.sendrawtransaction(tx)

        current_height = self.btcrpc.proxy.getblockchaininfo()["blocks"]

        self.info(f"Created deposit with txid: {dt_tx_id}")
        # Generate blocks to mature DT
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        # Wait for DT maturation
        strata_waiter = StrataWaiter(self.seqrpc, self.logger, timeout=30, interval=1)
        strata_waiter.wait_until_l1_height_at(current_height + 6)

        return dt_tx_id

    def fulfill_withdrawal_intents(self, ctx: flexitest.RunContext) -> list[str]:
        """
        Process withdrawal intents by creating Bitcoin withdrawal fulfillment transactions.
        Waits for withdrawal intents to be processed and removed from the list.
        Returns list of withdrawal fulfillment txids
        """
        btc_url = self.btcrpc.base_url
        btc_user = self.btc.get_prop("rpc_user")
        btc_password = self.btc.get_prop("rpc_password")

        # Get initial withdrawal intents from sequencer
        initial_withdrawal_intents = self.seqrpc.strata_getCurWithdrawalAssignments()
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
                    intent["destination"],
                    intent["amt"],
                    intent["operator_idx"],
                    intent["deposit_idx"],
                    intent["deposit_txid"],
                    btc_url,
                    btc_user,
                    btc_password,
                )

                tx_hex = bytes(tx).hex()
                wft_tx_id = self.btcrpc.proxy.sendrawtransaction(tx_hex)
                fulfillment_txids.append(wft_tx_id)

                self.info(f"Created withdrawal fulfillment txid: {wft_tx_id}")

            except Exception as e:
                self.error(f"Failed to create withdrawal fulfillment for intent {intent}: {e}")
                raise

        # Generate blocks to mature fulfillment transactions
        seq_addr = self.seq.get_prop("address")
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)

        # Wait for withdrawal intents to be processed and removed
        expected_final_count = initial_intent_count - len(fulfillment_txids)
        self.info(
            f"Waiting for withdrawal intents to be processed "
            f"(expecting {expected_final_count} remaining after being processed)"
        )

        def intent_waiter():
            intents = self.seqrpc.strata_getCurWithdrawalAssignments()
            return len(intents) <= expected_final_count

        wait_until(
            lambda: intent_waiter,
            error_with=(
                f"Timeout waiting for withdrawal intents to be processed "
                f"(expected <= {expected_final_count})"
            ),
            timeout=60,
            step=2,
        )

        final_intent_count = len(self.seqrpc.strata_getCurWithdrawalAssignments())
        self.info(
            f"Withdrawal fulfillment complete: {initial_intent_count} -> "
            f"{final_intent_count} intents"
        )

        return fulfillment_txids

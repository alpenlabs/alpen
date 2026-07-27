"""Test that the per-transaction Bitcoin-DA fee is charged and credited to the vault.

The DA fee is charged in-EVM as `da_rate * diff_size`, drawn from the caller's unused
(prepaid) gas budget and credited to the DA fee vault predeploy. Because it is bounded by
the unused gas, the transaction must be sent with gas headroom (gas_limit > gas_used) for
any fee to be charged.
"""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import ServiceType
from common.evm_utils import get_balance, wait_for_receipt
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

# The DA fee vault predeploy (see alpen-reth-evm constants::DA_FEE_VAULT_ADDRESS).
DA_FEE_VAULT = "0x5400000000000000000000000000000000000003"

# Non-zero DA rate (wei per byte) so the charge is active for this env only.
DA_RATE_WEI_PER_BYTE = 1000

TRANSFER_AMOUNT_WEI = 10**17

# Send with headroom above the 21000-gas transfer cost so there is an unused gas budget
# for the DA fee to be drawn from.
TX_GAS_LIMIT = 60000


@flexitest.register
class TestDaFee(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                da_rate_wei_per_byte=DA_RATE_WEI_PER_BYTE,
            )
        )

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)
        rpc = ee_sequencer.create_rpc()

        dev_account = get_dev_account(rpc)
        recipient = "0x000000000000000000000000000000000000dEaD"

        vault_before = get_balance(rpc, DA_FEE_VAULT)
        sender_before = get_balance(rpc, dev_account.address)
        recipient_before = get_balance(rpc, recipient)

        gas_price = int(rpc.eth_gasPrice(), 16)
        raw_tx = dev_account.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=TX_GAS_LIMIT,
        )

        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"

        gas_used = int(receipt["gasUsed"], 16)
        effective_gas_price = int(receipt["effectiveGasPrice"], 16)
        execution_fee = gas_used * effective_gas_price

        vault_after = get_balance(rpc, DA_FEE_VAULT)
        sender_after = get_balance(rpc, dev_account.address)
        recipient_after = get_balance(rpc, recipient)

        da_fee = vault_after - vault_before
        logger.info(f"DA fee credited to vault: {da_fee} wei")

        # The DA fee vault must have received a positive fee.
        assert da_fee > 0, "expected a positive DA fee credited to the vault"

        # The recipient got exactly the transfer.
        assert recipient_after - recipient_before == TRANSFER_AMOUNT_WEI

        # The sender paid: transfer value + execution gas fee + DA fee, and nothing more.
        sender_debit = sender_before - sender_after
        assert sender_debit == TRANSFER_AMOUNT_WEI + execution_fee + da_fee, (
            f"sender debit {sender_debit} != value + execution_fee + da_fee "
            f"({TRANSFER_AMOUNT_WEI} + {execution_fee} + {da_fee})"
        )

        # The DA fee must stay within the unused (prepaid) gas budget it is drawn from.
        unused_gas_budget = (TX_GAS_LIMIT - gas_used) * effective_gas_price
        assert da_fee <= unused_gas_budget, (
            f"DA fee {da_fee} exceeded the unused gas budget {unused_gas_budget}"
        )

        logger.info("DA fee charged and bounded correctly")
        return True

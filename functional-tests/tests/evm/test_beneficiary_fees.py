"""Test that gas fees are credited to the configured beneficiary address."""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import ServiceType
from common.evm_utils import get_balance, wait_for_receipt
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

# A fresh address with no genesis balance, distinct from the default beneficiary.
CUSTOM_BENEFICIARY = "0x1000000000000000000000000000000000000001"

TRANSFER_AMOUNT_WEI = 10**17


@flexitest.register
class TestBeneficiaryFees(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0, enable_l1_da=True, beneficiary_address=CUSTOM_BENEFICIARY
            )
        )

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)
        rpc = ee_sequencer.create_rpc()

        dev_account = get_dev_account(rpc)

        beneficiary_before = get_balance(rpc, CUSTOM_BENEFICIARY)
        assert beneficiary_before == 0, (
            f"Expected custom beneficiary to start at 0, got {beneficiary_before}"
        )

        gas_price = int(rpc.eth_gasPrice(), 16)
        recipient = "0x000000000000000000000000000000000000dEaD"

        raw_tx = dev_account.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=21000,
        )

        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"

        gas_used = int(receipt["gasUsed"], 16)
        effective_gas_price = int(receipt["effectiveGasPrice"], 16)
        expected_fee = gas_used * effective_gas_price

        beneficiary_after = get_balance(rpc, CUSTOM_BENEFICIARY)
        fee_received = beneficiary_after - beneficiary_before

        assert fee_received == expected_fee, (
            f"Beneficiary received {fee_received} wei, expected {expected_fee} "
            f"(gas_used={gas_used}, effective_gas_price={effective_gas_price})"
        )

        logger.info(f"Beneficiary {CUSTOM_BENEFICIARY} received {fee_received} wei in fees")
        return True

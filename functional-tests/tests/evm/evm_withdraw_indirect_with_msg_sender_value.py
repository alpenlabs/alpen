import logging

import flexitest

from envs import net_settings, testenv
from envs.rollup_params_cfg import RollupConfig
from mixins.bridge_out_precompile_contract_mixin import BridgePrecompileMixin
from utils.constants import SATS_TO_WEI


@flexitest.register
class ContractBridgeOutWithSenderValueTest(BridgePrecompileMixin):
    def __init__(self, ctx: flexitest.InitContext):
        fast_batch_settings = net_settings.get_fast_batch_settings()
        ctx.set_env(
            testenv.BasicEnvConfig(pre_generate_blocks=101, rollup_settings=fast_batch_settings)
        )

    def main(self, ctx: flexitest.RunContext):
        logging.warn("test temporarily disabled")
        return

        # deposit once
        self.deposit(ctx, self.bridge_eth_account.address, self.bridge_pk)

        cfg: RollupConfig = ctx.env.rollup_cfg()
        deposit_amount = cfg.deposit_amount

        # Call the contract function
        # TODO: use self.txs.deploy and self.txs.call
        contract_instance = self.w3.eth.contract(
            abi=self.abi, address=self.deployed_contract_receipt.contractAddress
        )
        tx_hash = contract_instance.functions.withdraw(self.bosd).transact(
            {"gas": 5_000_000, "value": deposit_amount * SATS_TO_WEI}
        )

        tx_receipt = self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)
        assert tx_receipt.status == 1

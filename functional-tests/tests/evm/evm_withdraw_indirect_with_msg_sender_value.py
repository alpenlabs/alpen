import flexitest

from envs import net_settings, testenv
from envs.rollup_params_cfg import RollupConfig
from mixins.bridge_out_precompile_contract_mixin import BridgePrecompileMixin
from utils import ProverClientSettings, get_priv_keys
from utils.constants import SATS_TO_WEI


@flexitest.register
class ContractBridgeOutWithSenderValueTest(BridgePrecompileMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
                auto_generate_blocks=True,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        priv_keys = get_priv_keys(ctx)

        # deposit twice
        self.deposit(ctx, self.web3.address, priv_keys)
        self.deposit(ctx, self.web3.address, priv_keys)
        print(self.web3.address)

        cfg: RollupConfig = ctx.env.rollup_cfg()
        deposit_amount = cfg.deposit_amount

        # Call the contract function
        # TODO: use self.txs.deploy and self.txs.call
        # check balance
        contract_instance = self.web3.eth.contract(
            abi=self.abi, address=self.deployed_contract_receipt.contractAddress
        )
        tx_hash = contract_instance.functions.withdraw(self.bosd).transact(
            {"gas": 5_000_000, "value": deposit_amount * SATS_TO_WEI}
        )

        tx_receipt = self.web3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)
        assert tx_receipt.status == 1

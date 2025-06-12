import flexitest

from envs import net_settings, testenv
from mixins.bridge_out_precompile_contract_mixin import BridgePrecompileMixin


@flexitest.register
class ContractBridgeOutWithNoValueTest(BridgePrecompileMixin):
    def __init__(self, ctx: flexitest.InitContext):
        fast_batch_settings = net_settings.get_fast_batch_settings()
        ctx.set_env(
            testenv.BasicEnvConfig(pre_generate_blocks=101, rollup_settings=fast_batch_settings)
        )

    def main(self, _ctx: flexitest.RunContext):
        # no need to deposit as we are just calling the contract with no value
        tx_receipt = self.txs.call_contract(
            self.withdraw_contract_id, "withdrawWithoutBalance", self.bosd
        )
        assert tx_receipt.status == 0

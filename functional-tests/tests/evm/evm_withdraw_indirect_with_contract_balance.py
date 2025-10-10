import flexitest

from envs import net_settings, testenv
from mixins.bridge_out_precompile_contract_mixin import BridgePrecompileMixin
from utils import ProverClientSettings, get_priv_keys


@flexitest.register
class ContractBridgeOutWithContractBalanceTest(BridgePrecompileMixin):
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
        # Deposit to contract Address
        priv_keys = get_priv_keys(ctx)
        self.deposit(ctx, self.deployed_contract_receipt.contractAddress, priv_keys)
        self.deposit(ctx, self.web3.address, priv_keys)

        # withdraw
        # TODO: use self.txs.deploy and self.txs.call
        contract_instance = self.w3.eth.contract(
            abi=self.abi, address=self.deployed_contract_receipt.contractAddress
        )
        tx_hash = contract_instance.functions.withdrawWithOwnBalance(self.bosd).transact(
            {"gas": 5_000_000}
        )

        tx_receipt = self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)
        assert tx_receipt.status == 1

import flexitest

from factory.test_cli import extract_p2tr_pubkey, xonlypk_to_descriptor
from mixins import bridge_mixin
from utils import get_bridge_pubkey
from utils.transaction import SmartContracts


class BridgePrecompileMixin(bridge_mixin.BridgeMixin):
    def premain(self, ctx: flexitest.InitContext):
        super().premain(ctx)

        self.withdraw_address = ctx.env.gen_ext_btc_address()
        self.bridge_pk = get_bridge_pubkey(self.seqrpc)
        self.w3.eth.default_account = self.w3.address

        xonlypk = extract_p2tr_pubkey(self.withdraw_address)
        bosd = xonlypk_to_descriptor(xonlypk)

        self.bosd = bytes.fromhex(bosd)

        # Extract ABI for compatibility with existing tests
        self.abi, _ = SmartContracts.compile_contract(
            "IndirectWithdrawalProxy.sol", "WithdrawCaller"
        )

        # Deploy contract.
        self.withdraw_contract_id = "withdraw_contract"
        contract_address, _ = self.txs.deploy_contract(
            "IndirectWithdrawalProxy.sol", "WithdrawCaller", self.withdraw_contract_id
        )

        # Create a simple object to hold the contract address for compatibility
        class DeploymentReceipt:
            def __init__(self, contract_address):
                self.contractAddress = contract_address

        self.deployed_contract_receipt = DeploymentReceipt(contract_address)

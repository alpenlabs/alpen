import flexitest
from solcx import install_solc, set_solc_version
from strata_utils import extract_p2tr_pubkey, xonlypk_to_descriptor

from load.reth.transaction import SmartContracts
from mixins import bridge_mixin
from utils import get_bridge_pubkey


class BridgePrecompileMixin(bridge_mixin.BridgeMixin):
    def premain(self, ctx: flexitest.InitContext):
        super().premain(ctx)

        install_solc(version="0.8.16")
        set_solc_version("0.8.16")

        self.withdraw_address = ctx.env.gen_ext_btc_address()
        self.el_address = self.eth_account.address
        self.bridge_pk = get_bridge_pubkey(self.seqrpc)
        self.web3.eth.default_account = self.web3.address
        self.contract = self.deploy_contract()
        xonlypk = extract_p2tr_pubkey(self.withdraw_address)
        bosd = xonlypk_to_descriptor(xonlypk)

        self.bosd = bytes.fromhex(bosd)

    def deploy_contract(self):
        """Compiles and deploys the contract, returning the instance and address."""
        self.abi, bytecode = SmartContracts.compile_contract(
            "BridgeOutPrecompile.sol", "BridgeOutCaller"
        )
        contract = self.web3.eth.contract(abi=self.abi, bytecode=bytecode)
        tx_hash = contract.constructor().transact()

        self.deployed_contract_receipt = self.web3.eth.wait_for_transaction_receipt(
            tx_hash, timeout=30
        )
        return self.web3.eth.contract(
            abi=self.abi, address=self.deployed_contract_receipt.contractAddress
        )

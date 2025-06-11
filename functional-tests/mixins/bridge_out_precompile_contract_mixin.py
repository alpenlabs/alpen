import flexitest
from solcx import install_solc, set_solc_version
from strata_utils import extract_p2tr_pubkey, xonlypk_to_descriptor

from mixins import bridge_mixin
from utils import get_bridge_pubkey


class BridgePrecompileMixin(bridge_mixin.BridgeMixin):
    def premain(self, ctx: flexitest.InitContext):
        super().premain(ctx)

        install_solc(version="0.8.16")
        set_solc_version("0.8.16")

        self.withdraw_address = ctx.env.gen_ext_btc_address()
        self.bridge_pk = get_bridge_pubkey(self.seqrpc)

        xonlypk = extract_p2tr_pubkey(self.withdraw_address)
        bosd = xonlypk_to_descriptor(xonlypk)

        self.bosd = bytes.fromhex(bosd)

        # Deploy contract.
        self.withdraw_contract_id = "withdraw_contract"
        self.txs.deploy_contract(
            "IndirectWithdrawalProxy.sol", "WithdrawCaller", self.withdraw_contract_id
        )

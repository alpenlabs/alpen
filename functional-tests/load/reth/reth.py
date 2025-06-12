import web3

from load.job import StrataLoadJob
from utils.evm_account import FundedAccount, GenesisAccount


class BaseRethLoadJob(StrataLoadJob):
    """
    Base class for all load jobs targeting Reth.
    """

    def before_start(self):
        super().before_start()
        self.w3 = self._new_w3()
        self.genesis_acc = GenesisAccount(self.w3)

    def new_account(self):
        new_acc = FundedAccount(self._new_w3())
        new_acc.fund_me(self.genesis_acc)
        return new_acc

    def _new_w3(self):
        return web3.Web3(web3.Web3.HTTPProvider(self.host, session=self.client))

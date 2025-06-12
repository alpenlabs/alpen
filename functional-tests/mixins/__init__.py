import flexitest
from web3 import Web3

from envs import testenv
from utils.evm_account import FundedAccount, GenesisAccount
from utils.transaction import EthTransactions


class BaseMixin(testenv.StrataTestBase):
    def premain(self, ctx: flexitest.RunContext):
        super().premain(ctx)
        self._ctx = ctx

        self.btc = ctx.get_service("bitcoin")
        self.seq = ctx.get_service("sequencer")
        self.seq_signer = ctx.get_service("sequencer_signer")
        self.reth = ctx.get_service("reth")

        self.seqrpc = self.seq.create_rpc()
        self.btcrpc = self.btc.create_rpc()
        self.rethrpc = self.reth.create_rpc()

        # Genesis account is from the genesis alloc.
        # It's only used to distribute funds to other accounts.
        genesis_account = GenesisAccount(self._new_w3())
        # Funded account is a fresh account with funds from genesis acc.
        # It's an account on behalf of which all the transactions are done.
        w3 = self._new_w3()
        funded_acc = FundedAccount(w3)
        funded_acc.fund_me(genesis_account)
        # Setting transactions api with default DEBUG level.
        self._txs = EthTransactions(funded_acc, self.debug)
        self._w3 = w3

    @property
    def txs(self) -> EthTransactions:
        """The main API to spawn various ETH transactions."""
        return self._txs

    @property
    def w3(self) -> Web3:
        """
        Web3 instance with the signing middleware configured for freshly funded acc.
        Corresponds to @property `txs`.
        """
        return self._w3

    def _new_w3(self):
        ethrpc_http_port = self.reth.get_prop("eth_rpc_http_port")
        http_ethrpc_url = f"http://localhost:{ethrpc_http_port}"
        return Web3(Web3.HTTPProvider(http_ethrpc_url))

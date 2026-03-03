"""Environment configurations."""

import subprocess
from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.config.params import GenesisAccountData, GenesisL1View, OLParams
from factories.bitcoin import BitcoinFactory
from factories.strata import StrataFactory


class StrataEnvConfig(flexitest.EnvConfig):
    """
    Strata environment: Initializes services required to run strata.

    Supports optional genesis accounts, epoch sealing config, and
    pre-funding the strata-test-cli BDK wallet for Bitcoin tx construction.
    """

    def __init__(
        self,
        pre_generate_blocks: int = 0,
        genesis_accounts: dict[str, GenesisAccountData] | None = None,
        epoch_sealing: EpochSealingConfig | None = None,
        fund_test_cli_wallet: bool = False,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.genesis_accounts = genesis_accounts
        self.epoch_sealing = epoch_sealing
        self.fund_test_cli_wallet = fund_test_cli_wallet

    def _fund_bdk_wallet(self, btc_rpc) -> None:
        """Pre-fund the strata-test-cli BDK wallet so it can build Bitcoin txs."""
        try:
            result = subprocess.run(
                ["strata-test-cli", "get-address", "--index", "0"],
                capture_output=True,
                text=True,
                timeout=30,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    f"strata-test-cli get-address failed (exit {result.returncode}):\n"
                    f"  stderr: {result.stderr.strip()}"
                )
            bdk_addr = result.stdout.strip()
        except FileNotFoundError:
            raise RuntimeError(
                "strata-test-cli binary not found. "
                "Ensure it is built with: cargo build --bin strata-test-cli"
            )
        btc_rpc.proxy.sendtoaddress(bdk_addr, 10)
        mine_addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(1, mine_addr)

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        services = self.get_services(ectx, self.pre_generate_blocks)
        return flexitest.LiveEnv(services)

    @staticmethod
    def get_services(ectx: flexitest.EnvContext, pre_generate_blocks: int = 0):
        btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))

        # Start Bitcoin
        bitcoind = btc_factory.create_regtest()
        bitcoind.wait_for_ready(timeout=10)

        # Create wallet and generate initial blocks
        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(pre_generate_blocks, addr)

        if self.fund_test_cli_wallet:
            self._fund_bdk_wallet(btc_rpc)

        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        # TODO: set up reth config

        genesis_l1 = GenesisL1View.at_latest_block(btc_rpc)

        # Build OL params with optional genesis accounts
        ol_params = None
        if self.genesis_accounts is not None:
            ol_params = OLParams(accounts=self.genesis_accounts).with_genesis_l1(genesis_l1)

        strata = strata_factory.create_node(
            bitcoind_config,
            genesis_l1,
            is_sequencer=True,
            ol_params=ol_params,
            epoch_sealing=self.epoch_sealing,
        )
        strata.wait_for_ready(timeout=30)

        services = {
            ServiceType.Bitcoin: bitcoind,
            ServiceType.Strata: strata,
        }
        return services

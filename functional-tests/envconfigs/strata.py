"""Environment configurations."""

import subprocess
from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.config.params import GenesisAccountData, GenesisL1View, OLParams
from factories.bitcoin import BitcoinFactory
from factories.signer import SignerFactory
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
        admin_confirmation_depth: int | None = None,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.genesis_accounts = genesis_accounts
        self.epoch_sealing = epoch_sealing
        self.fund_test_cli_wallet = fund_test_cli_wallet
        self.admin_confirmation_depth = admin_confirmation_depth

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
        except FileNotFoundError as err:
            raise RuntimeError(
                "strata-test-cli binary not found. "
                "Ensure it is built with: cargo build --bin strata-test-cli"
            ) from err
        btc_rpc.proxy.sendtoaddress(bdk_addr, 10)
        mine_addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(1, mine_addr)

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        services = self._get_services(ectx)
        return flexitest.LiveEnv(services)

    def _get_services(self, ectx: flexitest.EnvContext):
        btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        signer_factory = cast(SignerFactory, ectx.get_factory(ServiceType.StrataSigner))

        # Start Bitcoin
        bitcoind = btc_factory.create_regtest()
        bitcoind.wait_for_ready(timeout=10)

        # Create wallet and generate initial blocks
        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if self.pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(self.pre_generate_blocks, addr)

        if self.fund_test_cli_wallet:
            self._fund_bdk_wallet(btc_rpc)

        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        genesis_l1 = GenesisL1View.at_latest_block(btc_rpc)

        # Build OL params with optional genesis accounts
        ol_params = None
        if self.genesis_accounts is not None:
            ol_params = OLParams(accounts=self.genesis_accounts).with_genesis_l1(genesis_l1)

        # Start Strata sequencer
        strata, sequencer_key_path = strata_factory.create_node(
            bitcoind_config,
            genesis_l1.blk.height,
            is_sequencer=True,
            ol_params=ol_params,
            epoch_sealing_config=self.epoch_sealing,
            admin_confirmation_depth=self.admin_confirmation_depth,
        )
        strata.wait_for_ready(timeout=30)

        # Start strata-signer for the sequencer (connects to strata's WS RPC)
        assert sequencer_key_path is not None
        signer = signer_factory.create_signer(
            sequencer_key_path,
            strata.props["rpc_host"],
            strata.props["rpc_port"],
        )
        signer.wait_for_ready(timeout=10)

        services = {
            ServiceType.Bitcoin: bitcoind,
            ServiceType.Strata: strata,
            ServiceType.StrataSigner: signer,
        }
        return services

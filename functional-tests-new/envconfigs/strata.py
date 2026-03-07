"""Environment configurations."""

from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from factories.bitcoin import BitcoinFactory
from factories.strata import StrataFactory


class StrataEnvConfig(flexitest.EnvConfig):
    """
    Strata environment: Initializes services required to run strata.
    """

    def __init__(
        self,
        pre_generate_blocks: int = 0,
        epoch_slots: int | None = None,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.epoch_slots = epoch_slots

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        epoch_sealing_config = (
            EpochSealingConfig(slots_per_epoch=self.epoch_slots)
            if self.epoch_slots is not None
            else None
        )

        services = self.get_services(
            ectx,
            self.pre_generate_blocks,
            epoch_sealing_config=epoch_sealing_config,
        )
        return flexitest.LiveEnv(services)

    @staticmethod
    def get_services(
        ectx: flexitest.EnvContext,
        pre_generate_blocks: int = 0,
        epoch_sealing_config: EpochSealingConfig | None = None,
    ):
        btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))

        # Start Bitcoin
        bitcoind = btc_factory.create_regtest()

        # Wait for Bitcoin RPC to be ready
        bitcoind.wait_for_ready(timeout=10)

        # Create wallet and generate initial blocks
        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(pre_generate_blocks, addr)

        # Create config (props validated by dataclass at factory level)
        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        # TODO: set up reth config

        # Start Strata sequencer
        genesis_l1_height = btc_rpc.proxy.getblockcount()
        strata = strata_factory.create_node(
            bitcoind_config,
            genesis_l1_height,
            is_sequencer=True,
            epoch_sealing_config=epoch_sealing_config,
        )
        strata.wait_for_ready(timeout=10)

        services = {
            ServiceType.Bitcoin: bitcoind,
            ServiceType.Strata: strata,
        }
        return services

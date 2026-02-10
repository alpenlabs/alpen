"""Environment configurations."""

from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.config.params import GenesisL1View
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
        block_time_ms: int | None = None,
        proof_publish_mode: dict | None = None,
        checkpoint_predicate: str | None = None,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.epoch_slots = epoch_slots
        self.block_time_ms = block_time_ms
        self.proof_publish_mode = proof_publish_mode
        self.checkpoint_predicate = checkpoint_predicate

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))

        # Start Bitcoin
        bitcoind = btc_factory.create_regtest()

        # Wait for Bitcoin RPC to be ready
        bitcoind.wait_for_ready(timeout=10)

        # Create wallet and generate initial blocks
        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if self.pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(self.pre_generate_blocks, addr)

        # Create config (props validated by dataclass at factory level)
        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        # TODO: set up reth config

        # Start Strata sequencer
        genesis_l1 = GenesisL1View.at_latest_block(btc_rpc)
        rollup_params_overrides: dict[str, object] = {}
        if self.block_time_ms is not None:
            rollup_params_overrides["block_time"] = self.block_time_ms
        if self.proof_publish_mode is not None:
            rollup_params_overrides["proof_publish_mode"] = self.proof_publish_mode
        if self.checkpoint_predicate is not None:
            rollup_params_overrides["checkpoint_predicate"] = self.checkpoint_predicate

        epoch_sealing_config = (
            EpochSealingConfig(slots_per_epoch=self.epoch_slots)
            if self.epoch_slots is not None
            else None
        )

        strata = strata_factory.create_node(
            bitcoind_config,
            genesis_l1,
            is_sequencer=True,
            rollup_params_overrides=rollup_params_overrides or None,
            epoch_sealing_config=epoch_sealing_config,
        )
        strata.wait_for_ready(timeout=10)

        services = {
            ServiceType.Bitcoin: bitcoind,
            ServiceType.Strata: strata,
        }

        return flexitest.LiveEnv(services)

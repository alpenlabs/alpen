"""Strata sequencer + fullnode environment configuration."""

from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.config.params import L1BlockCommitment
from factories.bitcoin import BitcoinFactory
from factories.signer import SignerFactory
from factories.strata import StrataFactory


class StrataSequencerFullnodeEnvConfig(flexitest.EnvConfig):
    """Environment with one Strata sequencer and one Strata fullnode."""

    def __init__(self, pre_generate_blocks: int = 0, epoch_slots: int | None = None):
        self.pre_generate_blocks = pre_generate_blocks
        self.epoch_slots = epoch_slots

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        epoch_sealing_config = (
            EpochSealingConfig(slots_per_epoch=self.epoch_slots)
            if self.epoch_slots is not None
            else None
        )
        services = self.get_services(
            ectx, self.pre_generate_blocks, epoch_sealing_config=epoch_sealing_config
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
        signer_factory = cast(SignerFactory, ectx.get_factory(ServiceType.StrataSigner))

        bitcoind = btc_factory.create_regtest()
        bitcoind.wait_for_ready(timeout=10)

        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(pre_generate_blocks, addr)

        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        genesis_l1_block = L1BlockCommitment.at_latest_block(btc_rpc)
        sequencer_node = strata_factory.create_node(
            bitcoind_config,
            genesis_l1_block.height,
            is_sequencer=True,
            epoch_sealing_config=epoch_sealing_config,
        )
        sequencer = sequencer_node.service
        sequencer.wait_for_ready(timeout=20)

        # Start strata-signer for the sequencer only
        assert sequencer_node.sequencer_key_path is not None
        signer = signer_factory.create_signer(
            sequencer_node.sequencer_key_path,
            sequencer.props["admin_rpc_host"],
            sequencer.props["admin_rpc_port"],
            sequencer.props["admin_rpc_token"],
        )
        signer.wait_for_ready(timeout=10)

        fullnode_result = strata_factory.create_node(
            bitcoind_config,
            genesis_l1_block.height,
            is_sequencer=False,
        )
        fullnode = fullnode_result.service
        fullnode.wait_for_ready(timeout=20)

        return {
            ServiceType.Bitcoin: bitcoind,
            ServiceType.Strata: sequencer,
            ServiceType.StrataSigner: signer,
            ServiceType.StrataFullnode: fullnode,
        }

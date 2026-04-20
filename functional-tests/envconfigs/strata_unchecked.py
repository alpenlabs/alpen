"""Strata environment with CredRule::Unchecked."""

from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.config.params import GenesisL1View
from factories.bitcoin import BitcoinFactory
from factories.signer import SignerFactory
from factories.strata import StrataFactory


class StrataUncheckedEnvConfig(flexitest.EnvConfig):
    """
    Strata environment with ``CredRule::Unchecked``.

    The sequencer key is NOT embedded in rollup params, so signature
    verification is bypassed.  The signer still runs to fulfill block signing
    duties; it will never receive ``SignRevealTx`` duties because those are
    handled in-process by the btcio writer.
    """

    def __init__(self, pre_generate_blocks: int = 0):
        self.pre_generate_blocks = pre_generate_blocks

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        signer_factory = cast(SignerFactory, ectx.get_factory(ServiceType.StrataSigner))

        bitcoind = btc_factory.create_regtest()
        bitcoind.wait_for_ready(timeout=10)

        btc_rpc = bitcoind.create_rpc()
        btc_rpc.proxy.createwallet("testwallet")

        if self.pre_generate_blocks > 0:
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(self.pre_generate_blocks, addr)

        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )

        genesis_l1 = GenesisL1View.at_latest_block(btc_rpc)

        strata, sequencer_key_path = strata_factory.create_node(
            bitcoind_config,
            genesis_l1.blk.height,
            is_sequencer=True,
            use_unchecked_cred_rule=True,
            epoch_sealing_config=EpochSealingConfig.new_fixed_slot(4),
        )
        strata.wait_for_ready(timeout=30)

        assert sequencer_key_path is not None
        signer = signer_factory.create_signer(
            sequencer_key_path,
            strata.props["rpc_host"],
            strata.props["rpc_port"],
        )
        signer.wait_for_ready(timeout=10)

        return flexitest.LiveEnv(
            {
                ServiceType.Bitcoin: bitcoind,
                ServiceType.Strata: strata,
                ServiceType.StrataSigner: signer,
            }
        )

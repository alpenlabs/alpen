"""EE + OL environment with a checkpoint-sync OL node alongside the sequencer."""

from typing import cast

import flexitest

from common.config import BitcoindConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from envconfigs.alpen_client import AlpenClientEnv
from envconfigs.el_ol import EeOLEnv
from factories.strata import StrataFactory


class EeOLCheckpointSyncEnv(EeOLEnv):
    """`el_ol` plus a checkpoint-sync Strata node.

    The checkpoint-sync node reconstructs OL state from L1-buried checkpoints
    instead of executing OL blocks. The EE node stays wired to the sequencer
    (it must submit updates there); the test compares the checkpoint-sync
    node's reconstructed OL state against the sequencer's.
    """

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        strata_services = self.strata_config._get_services(ectx)
        sequencer = strata_services[ServiceType.Strata]
        bitcoin = strata_services[ServiceType.Bitcoin]

        checkpoint_node = self._start_checkpoint_node(ectx, bitcoin)

        alpen_services = AlpenClientEnv.get_services(
            ectx,
            self.alpen_env_params,
            bitcoin_service=bitcoin,
            ol_endpoint=sequencer.props["rpc_url"],
        )

        services = {
            **strata_services,
            **alpen_services,
            ServiceType.StrataCheckpointNode: checkpoint_node,
        }
        return flexitest.LiveEnv(services)

    def _start_checkpoint_node(
        self, ectx: flexitest.EnvContext, bitcoin: BitcoinService
    ) -> StrataService:
        """Starts a non-sequencer node reusing the sequencer's params."""
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        sequencer_node = self.strata_config.sequencer_node
        assert sequencer_node is not None

        bconfig = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoin.get_prop('rpc_port')}",
            rpc_user=bitcoin.get_prop("rpc_user"),
            rpc_password=bitcoin.get_prop("rpc_password"),
        )
        checkpoint_node = strata_factory.create_node(
            bconfig,
            sequencer_node.genesis_l1_height,
            is_sequencer=False,
            shared_params=sequencer_node.params,
        ).service
        checkpoint_node.wait_for_ready(timeout=30)
        return checkpoint_node

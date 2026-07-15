"""EE + OL environment with a checkpoint-sync OL node alongside the sequencer."""

from pathlib import Path
from typing import cast

import flexitest

from common.config import BitcoindConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from envconfigs.alpen_client import AlpenClientEnv
from envconfigs.el_ol import EeOLEnv
from factories.signer import SignerFactory
from factories.strata import CreateNodeResult, StrataFactory


class EeOLCheckpointSyncEnv(EeOLEnv):
    """`el_ol` plus a checkpoint-sync Strata node.

    The checkpoint-sync node reconstructs OL state from L1-buried checkpoints
    instead of executing OL blocks. The EE node reads OL state from the
    checkpoint-sync node and submits transactions to the sequencer.
    """

    def __init__(
        self,
        *args,
        provision_promotion: bool = False,
        provision_recovery_node: bool = False,
        **kwargs,
    ):
        super().__init__(*args, **kwargs)
        self.provision_promotion = provision_promotion
        self.provision_recovery_node = provision_recovery_node

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        strata_services = self.strata_config._get_services(ectx)
        sequencer: StrataService = strata_services[ServiceType.Strata]
        bitcoin: BitcoinService = strata_services[ServiceType.Bitcoin]
        sequencer_node = self.strata_config.sequencer_node
        assert sequencer_node is not None

        checkpoint_result = self._start_checkpoint_node(ectx, bitcoin)
        checkpoint_node = checkpoint_result.service

        alpen_services = AlpenClientEnv.get_services(
            ectx,
            self.alpen_env_params,
            bitcoin_service=bitcoin,
            ol_endpoint=checkpoint_node.props["rpc_url"],
            ol_submit_endpoint=sequencer.props["submit_rpc_url"],
            ol_submit_token=sequencer.props["submit_rpc_token"],
            ee_params_path=sequencer_node.params.ee_params,
        )

        services = {
            **strata_services,
            **alpen_services,
            ServiceType.StrataCheckpointNode: checkpoint_node,
        }

        if self.provision_promotion:
            services.update(self._provision_promotion_services(ectx, bitcoin, checkpoint_result))

        if self.provision_recovery_node:
            services[ServiceType.StrataRecoveryCheckpointNode] = self._provision_recovery_node(
                ectx, bitcoin
            )

        return flexitest.LiveEnv(services)

    def _start_checkpoint_node(
        self, ectx: flexitest.EnvContext, bitcoin: BitcoinService
    ) -> CreateNodeResult:
        """Starts a non-sequencer node reusing the sequencer's params."""
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        sequencer_node = self.strata_config.sequencer_node
        assert sequencer_node is not None

        bconfig = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoin.get_prop('rpc_port')}",
            rpc_user=bitcoin.get_prop("rpc_user"),
            rpc_password=bitcoin.get_prop("rpc_password"),
        )
        checkpoint_result = strata_factory.create_node(
            bconfig,
            sequencer_node.genesis_l1_height,
            is_sequencer=False,
            shared_params=sequencer_node.params,
            l1_reorg_safe_depth=self.strata_config.l1_reorg_safe_depth,
        )
        checkpoint_result.service.wait_for_ready(timeout=30)
        return checkpoint_result

    def _provision_promotion_services(
        self,
        ectx: flexitest.EnvContext,
        bitcoin: BitcoinService,
        checkpoint_result: CreateNodeResult,
    ) -> dict[ServiceType, object]:
        """Provisions dormant services used after the test stops the checkpoint node."""
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        signer_factory = cast(SignerFactory, ectx.get_factory(ServiceType.StrataSigner))
        sequencer_node = self.strata_config.sequencer_node
        assert sequencer_node is not None

        promoted = strata_factory.create_node(
            self._bitcoind_config(bitcoin),
            sequencer_node.genesis_l1_height,
            is_sequencer=True,
            epoch_sealing_config=self.strata_config.epoch_sealing,
            ol_block_time_ms=self.strata_config.ol_block_time_ms,
            l1_reorg_safe_depth=self.strata_config.l1_reorg_safe_depth,
            existing_datadir=checkpoint_result.service.props["datadir"],
            extra_args=["--bootstrap-from-checkpoint"],
            auto_start=False,
            service_type=ServiceType.StrataPromotedSequencer,
        ).service

        key_dir = Path(ectx.make_service_dir("promoted-sequencer-key"))
        copied_key_path = key_dir / "sequencer.key"
        promoted_signer = signer_factory.create_signer(
            copied_key_path,
            promoted.props["admin_rpc_host"],
            promoted.props["admin_rpc_port"],
            promoted.props["admin_rpc_token"],
            auto_start=False,
            service_type=ServiceType.StrataPromotedSigner,
        )

        return {
            ServiceType.StrataPromotedSequencer: promoted,
            ServiceType.StrataPromotedSigner: promoted_signer,
        }

    def _provision_recovery_node(
        self, ectx: flexitest.EnvContext, bitcoin: BitcoinService
    ) -> StrataService:
        """Provisions a dormant, empty checkpoint-sync node for the DA continuity recovery check."""
        strata_factory = cast(StrataFactory, ectx.get_factory(ServiceType.Strata))
        sequencer_node = self.strata_config.sequencer_node
        assert sequencer_node is not None

        return strata_factory.create_node(
            self._bitcoind_config(bitcoin),
            sequencer_node.genesis_l1_height,
            is_sequencer=False,
            shared_params=sequencer_node.params,
            l1_reorg_safe_depth=self.strata_config.l1_reorg_safe_depth,
            auto_start=False,
            service_type=ServiceType.StrataRecoveryCheckpointNode,
        ).service

    @staticmethod
    def _bitcoind_config(bitcoin: BitcoinService) -> BitcoindConfig:
        return BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoin.get_prop('rpc_port')}",
            rpc_user=bitcoin.get_prop("rpc_user"),
            rpc_password=bitcoin.get_prop("rpc_password"),
        )

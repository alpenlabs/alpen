"""OL environment with a checkpoint-sync node alongside the sequencer."""

from pathlib import Path
from typing import cast

import flexitest

from common.config import BitcoindConfig, EpochSealingConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from envconfigs.strata import StrataEnvConfig
from factories.signer import SignerFactory
from factories.strata import CreateNodeResult, StrataFactory


class CheckpointSyncEnv(flexitest.EnvConfig):
    """A Strata sequencer plus a checkpoint-sync Strata node.

    The checkpoint-sync node reconstructs OL state from L1-buried checkpoints
    instead of executing OL blocks.

    Parameters:
        pre_generate_blocks: How many bitcoin blocks to pre-generate
        provision_promotion: Provision a dormant sequencer + signer pair that a
            test can start after stopping the checkpoint node
        provision_recovery_node: Provision a dormant, empty checkpoint-sync node
            for the DA continuity recovery check
    """

    def __init__(
        self,
        pre_generate_blocks: int = 0,
        seal_epoch_slots: int | None = None,
        admin_confirmation_depth: int | None = None,
        ol_block_time_ms: int | None = None,
        l1_reorg_safe_depth: int | None = None,
        provision_promotion: bool = False,
        provision_recovery_node: bool = False,
    ):
        epoch_seal_config = (
            EpochSealingConfig.new_fixed_slot(seal_epoch_slots)
            if seal_epoch_slots
            else EpochSealingConfig()
        )

        self.strata_config = StrataEnvConfig(
            pre_generate_blocks=pre_generate_blocks,
            epoch_sealing=epoch_seal_config,
            admin_confirmation_depth=admin_confirmation_depth,
            ol_block_time_ms=ol_block_time_ms,
            l1_reorg_safe_depth=l1_reorg_safe_depth,
        )
        self.provision_promotion = provision_promotion
        self.provision_recovery_node = provision_recovery_node

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        strata_services = self.strata_config._get_services(ectx)
        bitcoin: BitcoinService = strata_services[ServiceType.Bitcoin]

        checkpoint_result = self._start_checkpoint_node(ectx, bitcoin)

        services = {
            **strata_services,
            ServiceType.StrataCheckpointNode: checkpoint_result.service,
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

        checkpoint_result = strata_factory.create_node(
            self._bitcoind_config(bitcoin),
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

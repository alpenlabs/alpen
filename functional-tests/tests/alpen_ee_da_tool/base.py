"""Shared base class for alpen-ee-da-tool functional tests."""

import flexitest

from common.base_test import StrataNodeTest
from common.config.constants import ServiceType
from common.services import AlpenClientService, BitcoinService
from envconfigs.el_ol import EeOLEnv


class AlpenEeDaToolTestBase(StrataNodeTest):
    """Shared env and service accessors for alpen-ee-da-tool functional tests."""

    BATCH_SEALING_BLOCK_COUNT = 30

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                batch_sealing_block_count=self.BATCH_SEALING_BLOCK_COUNT,
            )
        )

    def _services(self) -> tuple[BitcoinService, AlpenClientService]:
        return (
            self.get_service(ServiceType.Bitcoin),
            self.get_service(ServiceType.AlpenSequencer),
        )

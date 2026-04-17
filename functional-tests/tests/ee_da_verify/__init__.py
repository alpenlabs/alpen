"""Functional tests for ee-da-verify."""

import flexitest

from common.base_test import StrataNodeTest
from common.config.constants import ServiceType
from common.services import AlpenClientService, BitcoinService
from envconfigs.alpen_client import AlpenClientEnv


class EeDaVerifyTestBase(StrataNodeTest):
    """Shared env + service-accessor for ee-da-verify functional tests."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                fullnode_count=0,
                enable_l1_da=True,
                batch_sealing_block_count=30,
            )
        )

    def _services(self) -> tuple[BitcoinService, AlpenClientService]:
        return (
            self.get_service(ServiceType.Bitcoin),
            self.get_service(ServiceType.AlpenSequencer),
        )

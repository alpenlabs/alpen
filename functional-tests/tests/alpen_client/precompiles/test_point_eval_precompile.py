"""Verify that the EIP-4844 point evaluation precompile (0x0a) is disabled.

Alpen EVM uses a Berlin-based spec, so the Cancun point evaluation
precompile is not available. Calling it returns empty bytes (0x).
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.precompile import PRECOMPILE_POINT_EVALUATION, call_precompile
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

# Verbatim from the old test: a valid-looking point eval input that would
# succeed on Cancun but is irrelevant here since the precompile is absent.
POINT_EVAL_INPUT = (
    "c000000000000000000000000000000000000000000000000000000000000000"
    "0000000000000000000000000000000000000000000000000000000000000000"
    "0000000000000000000000000200000000000000000000000000000000000000"
    "0000000000000000000000000000000000000000000000000000000000000000"
    "00000000000000000000000000c0000000000000000000000000000000000000"
    "0000000000000000000000000000000000000000000000000000000000"
)


@flexitest.register
class TestPointEvalPrecompile(BaseTest):
    """Point evaluation precompile (0x0a) is disabled and returns 0x."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        logger.info("Calling point evaluation precompile (0x0a) — expect empty result")
        _tx_hash, result = call_precompile(rpc, PRECOMPILE_POINT_EVALUATION, POINT_EVAL_INPUT)

        assert result == "0x", (
            f"Point evaluation precompile should be disabled: expected '0x', got '{result}'"
        )

        logger.info("Confirmed: point evaluation precompile is disabled (returned 0x)")
        return True

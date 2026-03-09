"""Verify the Alpen Schnorr precompile at 0x5400...0002.

Tests both valid and invalid (mismatched) Schnorr signatures. Uses the
strata-test-cli binary for signature generation.

Migrated from functional-tests/tests/el_schnorr_precompile.py.
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.precompile import (
    PRECOMPILE_SCHNORR_ADDRESS,
    SCHNORR_TEST_SECRET_KEY,
    call_precompile,
    get_schnorr_precompile_input,
)
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)


@flexitest.register
class TestSchnorrPrecompile(BaseTest):
    """Schnorr precompile: valid signature returns 0x01, mismatched returns 0x00.

    Precompile input format: pubkey(32B) || msg_hash(32B) || signature(64B).
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        secret_key = SCHNORR_TEST_SECRET_KEY

        # --- Test 1: valid signature for "AlpenStrata" ---
        msg = "AlpenStrata"
        precompile_input = get_schnorr_precompile_input(secret_key, msg)
        logger.info(f"Test 1: valid Schnorr signature for '{msg}'")

        _tx_hash, result = call_precompile(rpc, PRECOMPILE_SCHNORR_ADDRESS, precompile_input)
        assert result == "0x01", f"Schnorr verification failed: expected '0x01', got '{result}'"
        logger.info("  Valid signature: OK (0x01)")

        # --- Test 2: mismatched signature ---
        # Sign a different message, then splice its pubkey+hash with the first
        # message's signature. The signature won't match the hash → 0x00.
        another_msg = "MakaluStrata"
        another_input = get_schnorr_precompile_input(secret_key, another_msg)
        logger.info(f"Test 2: mismatched signature (hash from '{another_msg}', sig from '{msg}')")

        # Input layout (hex chars): pubkey(64) || msg_hash(64) || signature(128)
        # Take pubkey+hash from another_input, signature from first input.
        modified_input = another_input[:-128] + precompile_input[-128:]

        _tx_hash, result = call_precompile(rpc, PRECOMPILE_SCHNORR_ADDRESS, modified_input)
        assert result == "0x00", f"Schnorr verification failed: expected '0x00', got '{result}'"
        logger.info("  Mismatched signature: OK (0x00)")

        logger.info("Schnorr precompile tests passed")
        return True

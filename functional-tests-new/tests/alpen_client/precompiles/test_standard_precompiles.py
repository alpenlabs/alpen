"""Verify the 8 standard EVM precompiles (0x01 - 0x08) via direct eth_call.

Test vectors are extracted from functional-tests/contracts/PrecompileTestContract.sol.
Each precompile is called directly with raw input (no Solidity contract needed).
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.precompile import (
    PRECOMPILE_ECADD,
    PRECOMPILE_ECMUL,
    PRECOMPILE_ECPAIRING,
    PRECOMPILE_ECRECOVER,
    PRECOMPILE_IDENTITY,
    PRECOMPILE_MODEXP,
    PRECOMPILE_RIPEMD160,
    PRECOMPILE_SHA256,
    eth_call_precompile,
    normalize_hex,
)
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)


def _pad32(val_hex: str) -> str:
    """Left-pad a hex value to 32 bytes (64 hex chars)."""
    return val_hex.rjust(64, "0")


# ---------------------------------------------------------------------------
# Test vectors from PrecompileTestContract.sol
# ---------------------------------------------------------------------------

# ecrecover: hash(32B) || v_padded(32B) || r(32B) || s(32B)
ECRECOVER_INPUT = (
    "456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3"
    + _pad32("1c")  # v = 28
    + "9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608"
    + "4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada"
)
ECRECOVER_EXPECTED = _pad32("7156526fbd7a3c72969b54f64e42c10fbb768c8a")

# sha256(0xFF)
SHA256_INPUT = "ff"
SHA256_EXPECTED = "a8100ae6aa1940d0b663bb31cd466142ebbdbd5187131b92d93818987832eb89"

# ripemd160(0xFF) — 20-byte output left-padded to 32 bytes
RIPEMD160_INPUT = "ff"
RIPEMD160_EXPECTED = _pad32("2c0c45d3ecab80fe060e5f1d7057cd2f8de5e557")

# identity(0xFF)
IDENTITY_INPUT = "ff"
IDENTITY_EXPECTED = "ff"

# modexp: 8^9 mod 10 = 8
# Format: base_len(32B) || exp_len(32B) || mod_len(32B) || base(32B) || exp(32B) || mod(32B)
MODEXP_INPUT = _pad32("20") + _pad32("20") + _pad32("20") + _pad32("8") + _pad32("9") + _pad32("a")
MODEXP_EXPECTED = _pad32("8")

# ecadd: P(1,2) + P(1,2)
ECADD_INPUT = _pad32("1") + _pad32("2") + _pad32("1") + _pad32("2")
ECADD_EXPECTED = (
    "030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd3"
    "15ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4"
)

# ecmul: P(1,2) * 2 — same result as ecadd(P,P)
ECMUL_INPUT = _pad32("1") + _pad32("2") + _pad32("2")
ECMUL_EXPECTED = ECADD_EXPECTED

# ecpairing: 12 field elements from PrecompileTestContract.sol
ECPAIRING_INPUT = (
    "2cf44499d5d27bb186308b7af7af02ac5bc9eeb6a3d147c186b21fb1b76e18da"
    "2c0f001f52110ccfe69108924926e45f0b0c868df0e7bde1fe16d3242dc715f6"
    "1fb19bb476f6b9e44e2a32234da8212f61cd63919354bc06aef31e3cfaff3ebc"
    "22606845ff186793914e03e21df544c34ffe2f2f3504de8a79d9159eca2d98d9"
    "2bd368e28381e8eccb5fa81fc26cf3f048eea9abfdd85d7ed3ab3698d63e4f90"
    "2fe02e47887507adf0ff1743cbac6ba291e66f59be6bd763950bb16041a0a85e"
    "0000000000000000000000000000000000000000000000000000000000000001"
    "30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd45"
    "1971ff0471b09fa93caaf13cbf443c1aede09cc4328f5a62aad45f40ec133eb4"
    "091058a3141822985733cbdddfed0fd8d6c104e9e9eff40bf5abfef9ab163bc7"
    "2a23af9a5ce2ba2796c1f4e453a370eb0af8c212d9dc9acd8fc02c2e907baea2"
    "23a8eb0b0996252cb548a4487da97b02422ebc0e834613f954de6c7e0afdc1fc"
)
ECPAIRING_EXPECTED = _pad32("1")


# Each case: (name, address, input_hex, expected_output_hex)
STANDARD_PRECOMPILE_CASES = [
    ("ecrecover", PRECOMPILE_ECRECOVER, ECRECOVER_INPUT, ECRECOVER_EXPECTED),
    ("sha256", PRECOMPILE_SHA256, SHA256_INPUT, SHA256_EXPECTED),
    ("ripemd160", PRECOMPILE_RIPEMD160, RIPEMD160_INPUT, RIPEMD160_EXPECTED),
    ("identity", PRECOMPILE_IDENTITY, IDENTITY_INPUT, IDENTITY_EXPECTED),
    ("modexp", PRECOMPILE_MODEXP, MODEXP_INPUT, MODEXP_EXPECTED),
    ("ecadd", PRECOMPILE_ECADD, ECADD_INPUT, ECADD_EXPECTED),
    ("ecmul", PRECOMPILE_ECMUL, ECMUL_INPUT, ECMUL_EXPECTED),
    ("ecpairing", PRECOMPILE_ECPAIRING, ECPAIRING_INPUT, ECPAIRING_EXPECTED),
]


@flexitest.register
class TestStandardPrecompiles(BaseTest):
    """Verify 8 standard EVM precompiles (0x01-0x08) via direct eth_call."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        for name, address, input_hex, expected in STANDARD_PRECOMPILE_CASES:
            logger.info(f"Testing {name} at {address}")
            result = eth_call_precompile(rpc, address, input_hex)

            result_norm = normalize_hex(result)
            expected_norm = normalize_hex(expected)

            assert result_norm == expected_norm, (
                f"{name} failed: expected '{expected_norm}', got '{result_norm}'"
            )
            logger.info(f"  {name}: OK")

        logger.info("All 8 standard precompiles verified")
        return True

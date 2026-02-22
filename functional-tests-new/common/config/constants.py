"""
Constants used throughout the functional test suite.
"""

from enum import Enum

# =============================================================================
# EVM Dev Accounts
# =============================================================================
# Standard Foundry/Hardhat dev accounts with known private keys.
# These are pre-funded in dev chain configurations.

# Dev account #0 (genesis prefunded account)
DEV_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEV_ADDRESS = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

# Dev account #1 (recipient for tests)
DEV_RECIPIENT_PRIVATE_KEY = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
DEV_RECIPIENT_ADDRESS = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

# Chain ID for alpen-dev-chain
DEV_CHAIN_ID = 2892

# =============================================================================
# Protocol Addresses
# =============================================================================
# System addresses for fee distribution in Alpen EVM.
# These are hardcoded in the chain spec.
BASEFEE_ADDRESS = "0x5400000000000000000000000000000000000010"
BENEFICIARY_ADDRESS = "0x5400000000000000000000000000000000000011"

# =============================================================================
# Unit Conversions
# =============================================================================
SATS_TO_WEI = 10_000_000_000
GWEI_TO_WEI = 1_000_000_000

# =============================================================================
# Service Types
# =============================================================================


class ServiceType(str, Enum):
    """
    Service type identifiers for test environments.

    Using str Enum allows direct string comparison while providing
    IDE autocomplete and type safety.

    Usage:
        services = {ServiceType.Bitcoin: bitcoind, ServiceType.Strata: strata}
        bitcoin = self.get_service(ServiceType.Bitcoin)
    """

    AlpenClient = "alpen_client"
    Bitcoin = "bitcoin"
    Strata = "strata"

    def __str__(self) -> str:
        """Allow direct use in f-strings and format operations."""
        return self.value

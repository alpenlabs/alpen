"""
Constants used throughout the functional test suite.
"""

from enum import Enum


class ServiceType(str, Enum):
    """
    Service type identifiers for test environments.

    Using str Enum allows direct string comparison while providing
    IDE autocomplete and type safety.

    Usage:
        services = {ServiceType.Bitcoin: bitcoind, ServiceType.Strata: strata}
        bitcoin = self.get_service(ServiceType.Bitcoin)
    """

    Bitcoin = "bitcoin"
    Strata = "strata"

    def __str__(self) -> str:
        """Allow direct use in f-strings and format operations."""
        return self.value

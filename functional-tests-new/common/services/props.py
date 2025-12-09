"""
Service property dataclasses for type-safe service configuration.
"""

from dataclasses import dataclass
from pathlib import Path


@dataclass
class BitcoinServiceProps:
    """Properties for Bitcoin service."""

    rpc_port: int
    rpc_user: str
    rpc_password: str
    rpc_url: str
    p2p_port: int
    datadir: str | Path
    walletname: str = "testwallet"


@dataclass
class StrataServiceProps:
    """
    Properties for Strata service.

    All fields are required to ensure services are properly configured.
    """

    rpc_port: int
    rpc_host: str
    rpc_url: str
    datadir: str | Path
    mode: str  # "sequencer" or "fullnode"

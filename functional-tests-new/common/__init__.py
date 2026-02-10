"""
Core library for functional tests.
Provides service management, RPC clients, and waiting utilities.
"""

from .config import BitcoindConfig, RethELConfig, StrataConfig
from .rpc import JsonRpcClient, RpcError
from .wait import (
    wait_for_confirmed_epoch,
    wait_for_finalized_epoch,
    wait_until,
)

__all__ = [
    "JsonRpcClient",
    "RpcError",
    "wait_until",
    "wait_for_confirmed_epoch",
    "wait_for_finalized_epoch",
    "BitcoindConfig",
    "RethELConfig",
    "StrataConfig",
]

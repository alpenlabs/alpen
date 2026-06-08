"""Node public RPC surfaces match node capabilities."""

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc import JsonRpcClient, RpcError
from common.services.strata import StrataService

DUMMY_TRANSACTION = {}


@flexitest.register
class TestPublicRpcSurface(BaseTest):
    """Checks role-specific public RPC method exposure."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_checkpoint_sync")

    def main(self, ctx):
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        checkpoint_node: StrataService = self.get_service(ServiceType.StrataCheckpointNode)

        sequencer_rpc = sequencer.wait_for_rpc_ready(timeout=20)
        checkpoint_rpc = checkpoint_node.wait_for_rpc_ready(timeout=20)

        # Use an inverted range to avoid depending on available block data; any
        # error other than method-not-found proves the method is registered.
        assert_method_registered(sequencer_rpc, "strata_getRawBlocksRange", 1, 0)
        assert_method_not_found(sequencer_rpc, "strata_submitTransaction", DUMMY_TRANSACTION)
        assert_method_not_found(sequencer_rpc, "strata_strataadmin_getSequencerDuties")

        assert_method_not_found(checkpoint_rpc, "strata_getRawBlocksRange", 1, 0)
        assert_method_registered(
            checkpoint_rpc,
            "strata_getBlocksSummaries",
            ALPEN_ACCOUNT_ID,
            1,
            0,
        )
        assert_method_not_found(
            checkpoint_rpc,
            "strata_getSnarkAcctUpdateManifest",
            ALPEN_ACCOUNT_ID,
            0,
        )
        assert_method_not_found(checkpoint_rpc, "strata_submitTransaction", DUMMY_TRANSACTION)
        assert_method_not_found(
            checkpoint_rpc,
            "strata_getSnarkAccountState",
            ALPEN_ACCOUNT_ID,
            "latest",
        )
        assert_method_registered(checkpoint_rpc, "strata_getChainStatus")


def assert_method_registered(rpc: JsonRpcClient, method: str, *params) -> None:
    """Asserts that an RPC method is registered on this endpoint."""
    try:
        rpc.call(method, *params)
    except RpcError as err:
        assert err.code != -32601, f"{method} should be registered, got {err}"


def assert_method_not_found(rpc: JsonRpcClient, method: str, *params) -> None:
    """Asserts that an RPC method is not registered on this endpoint."""
    try:
        rpc.call(method, *params)
    except RpcError as err:
        assert err.code == -32601, f"expected method-not-found for {method}, got {err}"
        return

    raise AssertionError(f"{method} unexpectedly exists")

"""Basic node functionality tests."""

import flexitest

from common.base_test import BaseTest
from common.service import ServiceWrapper


# NOTE: this is redundant and is just for setting up the func tests infra. Remove later.
@flexitest.register
class TestNodeVersion(BaseTest):
    """Test that node starts and responds to protocolVersion calls."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        # Get services
        bitcoin: ServiceWrapper = ctx.get_service("bitcoin")
        strata: ServiceWrapper = ctx.get_service("strata")
        print(bitcoin)

        # Create RPC clients
        strata_rpc = strata.create_rpc()

        self.info("Waiting for Strata RPC to be ready...")
        self.wait_for_rpc_ready(strata_rpc, timeout=5)

        # Test protocol version
        self.info("Checking protocol version...")
        version = strata_rpc.strata_protocolVersion()
        self.info(f"Protocol version: {version}")
        assert version == 1, f"Expected version 1, got {version}"

        self.info("Test passed!")
        return True

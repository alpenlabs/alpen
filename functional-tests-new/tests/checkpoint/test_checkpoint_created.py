"""Verify that checkpoint entries are created for epoch >= 1."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from tests.checkpoint.helpers import (
    parse_checkpoint_epoch,
    wait_for_checkpoint_duty,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointCreated(StrataNodeTest):
    """Checkpoint entries for epoch >= 1 are created."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        btc_rpc = bitcoin.create_rpc()
        addr = btc_rpc.proxy.getnewaddress()

        # Drive L1 forward so OL can produce blocks and complete epoch 1.
        btc_rpc.proxy.generatetoaddress(5, addr)

        # Wait for OL to reach the terminal slot of epoch 1.
        epoch_sealing = strata.props["epoch_sealing"]
        assert epoch_sealing is not None, "checkpoint env must set epoch_sealing"
        assert epoch_sealing["policy"] == "FixedSlot", "test assumes FixedSlot policy"
        slots_per_epoch = epoch_sealing["slots_per_epoch"]
        assert slots_per_epoch is not None and slots_per_epoch > 0
        epoch1_terminal_slot = slots_per_epoch
        strata.wait_for_block_height(
            epoch1_terminal_slot, strata_rpc, timeout=120, poll_interval=0.5
        )

        # Wait for a checkpoint duty for epoch >= 1.
        duty = wait_for_checkpoint_duty(strata_rpc, timeout=60, step=0.5, min_epoch=1)
        epoch = parse_checkpoint_epoch(duty)
        assert epoch >= 1, f"expected checkpoint duty for epoch >= 1, got {epoch}"
        logger.info("Checkpoint created for epoch %d", epoch)

        return True

"""
Verify that checkpoint entries are created for epoch 0 (genesis) and epoch >= 1.

The checkpoint builder processes epoch 0 as catch-up when the first chain-worker
epoch notification fires (epoch 1 terminal).  We verify both by polling the
sequencer duty endpoint which only surfaces a SignCheckpoint duty when an
unsigned checkpoint entry exists in the database.
"""

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
    """Checkpoint entries for epoch 0 and epoch >= 1 are created."""

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

        # Wait for OL to reach the terminal block of epoch 1.
        epoch_sealing = strata.props["epoch_sealing"]
        assert epoch_sealing is not None, "checkpoint env must set epoch_sealing"
        assert epoch_sealing["policy"] == "FixedSlot", "test assumes FixedSlot policy"
        epoch1_terminal_slot = 1 * epoch_sealing["slots_per_epoch"]
        strata.wait_for_block_height(
            epoch1_terminal_slot, strata_rpc, timeout=120, poll_interval=0.5
        )

        # --- Epoch 0 checkpoint ---
        duty = wait_for_checkpoint_duty(strata_rpc, timeout=30, step=0.5)
        epoch = parse_checkpoint_epoch(duty)
        assert epoch == 0, f"expected first checkpoint duty for epoch 0, got {epoch}"
        logger.info("Checkpoint created for epoch %d", epoch)

        # --- Epoch >= 1 checkpoint ---
        # After the sequencer auto-signs epoch 0, the next duty is epoch >= 1.
        duty = wait_for_checkpoint_duty(strata_rpc, timeout=60, step=0.5, min_epoch=1)
        epoch = parse_checkpoint_epoch(duty)
        assert epoch >= 1, f"expected checkpoint duty for epoch >= 1, got {epoch}"
        logger.info("Checkpoint created for epoch %d", epoch)

        return True

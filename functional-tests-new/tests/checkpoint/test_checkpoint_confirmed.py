"""
Verify that signed checkpoints are posted to L1 and the ASM confirms them.

Path:
  epoch 1 terminal block -> checkpoint built -> proof timeout -> signed
  -> btcio posts commit+reveal txs to L1 -> mine L1 blocks
  -> ASM processes checkpoint -> confirmed epoch advances to >= 1
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_for_confirmed_epoch
from tests.checkpoint.helpers import (
    wait_for_checkpoint_duty,
    wait_for_no_unsigned_checkpoints,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointConfirmed(StrataNodeTest):
    """Signed checkpoint posted to L1, ASM confirms it, confirmed epoch advances."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        btc_rpc = bitcoin.create_rpc()

        # Drive L1 forward so OL can produce blocks up to epoch 1 terminal.
        addr = btc_rpc.proxy.getnewaddress()
        btc_rpc.proxy.generatetoaddress(5, addr)

        # Wait for terminal block of epoch 1.
        epoch_sealing = strata.props["epoch_sealing"]
        assert epoch_sealing is not None, "checkpoint env must set epoch_sealing"
        assert epoch_sealing["policy"] == "FixedSlot", "test assumes FixedSlot policy"
        epoch1_terminal_slot = 1 * epoch_sealing["slots_per_epoch"]
        strata.wait_for_block_height(
            epoch1_terminal_slot, strata_rpc, timeout=120, poll_interval=0.5
        )

        # 1. Wait for checkpoint builder to create at least one entry, then
        #    wait for the sequencer to sign all pending checkpoints.
        wait_for_checkpoint_duty(strata_rpc, timeout=30, step=0.5)
        wait_for_no_unsigned_checkpoints(strata_rpc, timeout=30, step=0.5)
        logger.info("All checkpoints signed, no unsigned duties remain")

        # 2. Mine L1 blocks one at a time to drive the commit->reveal cycle.
        #    btcio posts commit tx, waits for confirmation, then posts reveal tx.
        #    After each block, check whether the ASM has confirmed epoch 1.
        for i in range(20):
            btc_rpc.proxy.generatetoaddress(1, addr)
            try:
                epoch = wait_for_confirmed_epoch(strata_rpc, target_epoch=1, timeout=3, step=0.5)
                logger.info("Confirmed epoch %d after %d extra L1 blocks", epoch, i + 1)
                break
            except AssertionError:
                pass
        else:
            raise AssertionError("confirmed epoch did not reach 1 after 20 L1 blocks")

        return True

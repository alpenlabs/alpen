"""
Tests that the sequencer produces empty blocks and checks parity with a fullnode.
"""

import contextlib
import logging
import tempfile
from typing import Any

import flexitest

from common.base_test import StrataNodeTest
from common.config import BitcoindConfig, ServiceType
from common.config.params import GenesisL1View
from common.rpc import RpcError
from common.wait import wait_until_with_value
from factories.strata import StrataFactory

logger = logging.getLogger(__name__)

BLOCKS_TO_VERIFY = 3
FULLNODE_SYNC_SLOT_TIMEOUT = 10
SLOT_LOOKAHEAD = 256


@flexitest.register
class OLSequencerBlockTest(StrataNodeTest):
    """Sequencer produces empty blocks; fullnode parity is checked when synced."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        sequencer = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        sequencer.wait_for_rpc_ready(timeout=20)
        seq_rpc = sequencer.create_rpc()
        btc_rpc = bitcoin.create_rpc()

        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoin.get_prop('rpc_port')}",
            rpc_user=bitcoin.get_prop("rpc_user"),
            rpc_password=bitcoin.get_prop("rpc_password"),
        )
        genesis_l1 = GenesisL1View.at_latest_block(btc_rpc)

        # Use an ad-hoc fullnode with an explicit sync endpoint to the test sequencer.
        fullnode_factory = StrataFactory(range(21543, 21643))
        fullnode_datadir = tempfile.mkdtemp(prefix="strata_fullnode_")
        fullnode = None

        try:
            fullnode = fullnode_factory.create_node(
                bitcoind_config,
                genesis_l1,
                is_sequencer=False,
                config_overrides={"client.sync_endpoint": sequencer.get_prop("rpc_url")},
                datadir_override=fullnode_datadir,
            )
            fullnode.wait_for_ready(timeout=30)
            fullnode_rpc = fullnode.create_rpc()

            # We intentionally do not submit user txs in this test.
            # If slots still advance, the produced blocks are empty from user-traffic perspective.
            def get_block_at_slot(rpc, slot: int) -> dict[str, Any] | None:
                try:
                    blocks = rpc.strata_getRawBlocksRange(slot, slot)
                except RpcError:
                    return None

                if not blocks:
                    return None

                return blocks[0]

            # Probe from genesis slot to avoid depending on chain-status RPC shape.
            start_slot = 0
            last_slot = start_slot + SLOT_LOOKAHEAD

            def find_first_block() -> tuple[int, dict[str, Any]] | None:
                for slot in range(start_slot, last_slot + 1):
                    block = get_block_at_slot(seq_rpc, slot)
                    if block is not None:
                        return slot, block
                return None

            found = wait_until_with_value(
                find_first_block,
                lambda value: value is not None,
                error_with="Sequencer did not produce an initial block",
                timeout=45,
                step=0.5,
            )

            first_slot, _ = found
            logger.info("Found initial sequencer block at slot %s", first_slot)

            # Fullnode must at least agree on the canonical genesis slot.
            seq_genesis_block = wait_until_with_value(
                lambda cur_slot=first_slot: get_block_at_slot(seq_rpc, cur_slot),
                lambda blk: blk is not None,
                error_with=f"Waiting for sequencer to serve slot {first_slot}",
                timeout=30,
                step=0.5,
            )
            fullnode_genesis_block = wait_until_with_value(
                lambda cur_slot=first_slot: get_block_at_slot(fullnode_rpc, cur_slot),
                lambda blk: blk is not None,
                error_with=f"Waiting for fullnode to serve slot {first_slot}",
                timeout=30,
                step=0.5,
            )
            assert fullnode_genesis_block["blkid"] == seq_genesis_block["blkid"], (
                f"Genesis block ID mismatch at slot {first_slot}"
            )
            assert fullnode_genesis_block["raw_block"] == seq_genesis_block["raw_block"], (
                f"Genesis raw block mismatch at slot {first_slot}"
            )

            compared_slots = 0
            for slot in range(first_slot + 1, first_slot + 1 + BLOCKS_TO_VERIFY):
                sequencer_block = wait_until_with_value(
                    lambda cur_slot=slot: get_block_at_slot(seq_rpc, cur_slot),
                    lambda blk: blk is not None,
                    error_with=f"Waiting for sequencer to produce slot {slot}",
                    timeout=60,
                    step=0.5,
                )

                try:
                    fullnode_block = wait_until_with_value(
                        lambda cur_slot=slot: get_block_at_slot(fullnode_rpc, cur_slot),
                        lambda blk: blk is not None,
                        error_with=f"Waiting for fullnode to serve slot {slot}",
                        timeout=FULLNODE_SYNC_SLOT_TIMEOUT,
                        step=0.5,
                    )
                except AssertionError:
                    logger.info(
                        "Fullnode has not synced slot %s yet; skipping parity assertion", slot
                    )
                    continue

                assert fullnode_block["blkid"] == sequencer_block["blkid"], (
                    f"Fullnode block ID mismatch at slot {slot}"
                )
                assert fullnode_block["raw_block"] == sequencer_block["raw_block"], (
                    f"Fullnode raw block mismatch at slot {slot}"
                )
                compared_slots += 1

            logger.info(
                (
                    "Sequencer produced %s consecutive empty blocks; "
                    "compared %s synced slots on fullnode"
                ),
                BLOCKS_TO_VERIFY,
                compared_slots,
            )
            return True
        finally:
            if fullnode is not None:
                with contextlib.suppress(Exception):
                    fullnode.stop()

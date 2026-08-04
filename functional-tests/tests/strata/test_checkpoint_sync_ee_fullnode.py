"""An EE full node tracks finalized OL state through a checkpoint-sync node."""

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.wait import wait_until_with_value

# Leave enough blocks for the EE sequencer to produce a checkpoint containing
# the target EE block before we drive it to finality on L1.
TARGET_EE_BLOCK = 5


@flexitest.register
class TestCheckpointSyncEeFullnode(BaseTest):
    """Checks EE block sync and finality when only the full node reads CSS."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_checkpoint_sync")

    def main(self, ctx):
        ee_sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        ee_fullnode: AlpenClientService = self.get_service(ServiceType.AlpenFullNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        ee_sequencer.wait_for_block(TARGET_EE_BLOCK, timeout=120)
        target = ee_sequencer.get_block_by_number(TARGET_EE_BLOCK)
        assert target is not None, f"missing EE block {TARGET_EE_BLOCK} on sequencer"
        target_hash = target["hash"]

        # The full node gets execution blocks from the EE sequencer peer, even
        # though it derives their OL finality through the CSS OL endpoint.
        ee_fullnode.wait_for_block_hash(TARGET_EE_BLOCK, target_hash, timeout=120)

        sequencer_status = wait_until_with_value(
            lambda: _mine_and_get_block_status(ee_sequencer, target_hash, btc_rpc),
            lambda status: status["status"] == "finalized",
            error_with="EE sequencer did not finalize the target block",
            timeout=180,
        )
        fullnode_status = wait_until_with_value(
            lambda: _mine_and_get_block_status(ee_fullnode, target_hash, btc_rpc),
            lambda status: status["status"] == "finalized",
            error_with="CSS-backed EE full node did not finalize the target block",
            timeout=180,
        )

        assert sequencer_status["checkpoint_epoch"] == fullnode_status["checkpoint_epoch"]


def _mine_and_get_block_status(node: AlpenClientService, block_hash: str, btc_rpc) -> dict:
    btc_rpc.proxy.generatetoaddress(2, btc_rpc.proxy.getnewaddress())
    return node.get_block_status(block_hash)

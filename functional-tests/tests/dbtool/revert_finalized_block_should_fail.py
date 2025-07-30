import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import DbtoolMixin
from utils.dbtool import send_tx
from utils.utils import (
    ProverClientSettings,
    wait_for_genesis,
    wait_until_epoch_finalized,
)


@flexitest.register
class RevertFinalizedBlockShouldFailTest(DbtoolMixin):
    """Test that reverting to finalized blocks fails as expected"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Wait for genesis and generate some initial blocks
        wait_for_genesis(self.seqrpc, timeout=20)

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(self.web3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        wait_until_epoch_finalized(self.seqrpc, 1, timeout=30)

        # Generate more blocks to have a longer chain beyond the finalized epoch
        for _ in range(10):
            send_tx(self.web3)

        # Wait for both services to be in sync
        ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {ol_block_number}, EL block number: {el_block_number}")

        # Check if both services are at the same state before proceeding
        if ol_block_number != el_block_number:
            self.warning(f"OL and EL are not in sync: OL={ol_block_number}, EL={el_block_number}")

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        self.info("Testing that reverting a finalized block fails")

        # Get syncinfo to find finalized epoch
        self.info("Getting syncinfo to find finalized epoch")
        sync_info = self.get_syncinfo()

        finalized_epoch = sync_info.get("finalized_epoch", {})
        finalized_epoch_last_slot = finalized_epoch.get("last_slot", 0)
        finalized_epoch_last_blkid = finalized_epoch.get("last_blkid", "")

        self.info(f"Finalized epoch last slot: {finalized_epoch_last_slot}")

        if finalized_epoch_last_slot == 0:
            self.info("No finalized epoch yet, skipping this test")
            return True

        # Target a block that's BEFORE the finalized epoch (should fail)
        target_slot = finalized_epoch_last_slot - 1
        self.info(f"Targeting slot {target_slot}")

        # Get the L2 block to find the previous block ID
        l2_block_info = self.get_l2_block(finalized_epoch_last_blkid)

        # First level header is a SignedL2BlockHeader, then L2BlockHeader
        l2_block_header = l2_block_info.get("header", {}).get("header", {})
        target_block_id = l2_block_header.get("prev_block")

        if not target_block_id:
            self.error("No previous block ID found in L2 block info")
            return False

        # Try to revert to target_block_id (should fail)
        self.info(
            f"Attempting to revert to slot: {target_slot}, "
            f"block id: {target_block_id} (should fail)"
        )
        return_code, stdout, stderr = self.revert_chainstate(target_block_id)

        # The command should fail with an error
        if return_code == 0:
            self.error("revert-chainstate should have failed but succeeded")
            self.error(f"Stderr: {stdout}")
            return False

        self.info("Reverting to a block inside finalized epoch fails as expected")
        self.info(f"Stderr: {stderr}")
        return True

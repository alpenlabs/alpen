import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import SequencerDbtoolMixin
from utils.dbtool import send_tx
from utils.utils import ProverClientSettings


@flexitest.register
class DbtoolValidateSyncinfoTest(SequencerDbtoolMixin):
    """Test that sync info is valid and expected blocks/checkpoints exist"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                108,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq_waiter = self.create_strata_waiter(self.seqrpc)
        reth_waiter = self.create_reth_waiter(self.rethrpc)

        # Wait for genesis and generate some initial blocks
        seq_waiter.wait_until_genesis()

        # Generate some transactions to create blocks
        for _ in range(10):
            send_tx(self.w3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        seq_waiter.wait_until_epoch_finalized(0, timeout=30)

        # Generate more blocks to have a longer chain
        for _ in range(5):
            send_tx(self.w3)

        # Wait for more blocks to be generated
        reth_waiter.wait_until_eth_block_exceeds(10)

        # Stop services to ensure database is not being modified
        self.seq.stop()
        self.reth.stop()

        # Test 1: Get sync info and validate L1/L2 chain positions
        self.info("Testing get-syncinfo to validate chain positions")
        sync_info = self.get_syncinfo()

        # Verify we have reasonable chain positions
        l1_tip_height = sync_info.get("l1_tip_height", 0)
        l2_tip_height = sync_info.get("l2_tip_height", 0)

        if l1_tip_height <= 0:
            self.error(f"L1 tip height should be positive, got: {l1_tip_height}")
            return False

        if l2_tip_height < 0:
            self.error(f"L2 tip height should be non-negative, got: {l2_tip_height}")
            return False

        self.info(f"Sync validation passed - L1 tip: {l1_tip_height}, L2 tip: {l2_tip_height}")

        # Test 2: Verify L1 blocks exist
        self.info("Testing get-l1-summary to verify L1 blocks exist")
        l1_summary = self.get_l1_summary()

        expected_block_count = l1_summary.get("expected_block_count", 0)
        all_manifests_present = l1_summary.get("all_manifests_present", False)

        if expected_block_count <= 0:
            self.error(f"Should have expected L1 blocks, got: {expected_block_count}")
            return False

        if not all_manifests_present:
            self.error(f"All L1 manifests should be present, got: {all_manifests_present}")
            return False

        self.info(
            f"L1 blocks validation passed - {expected_block_count} expected blocks, "
            f"all manifests present"
        )

        # Test 3: Verify L2 blocks exist
        self.info("Testing get-l2-summary to verify L2 blocks exist")
        l2_summary = self.get_l2_summary()

        tip_slot = l2_summary.get("tip_slot", 0)
        all_blocks_present = l2_summary.get("all_blocks_present", False)

        if tip_slot <= 0:
            self.error(f"Should have L2 blocks, got tip_slot: {tip_slot}")
            return False

        if not all_blocks_present:
            self.error(f"All L2 blocks should be present, got: {all_blocks_present}")
            return False

        self.info(f"L2 blocks validation passed - tip slot: {tip_slot}, all blocks present")

        # Test 4: Verify checkpoints exist
        self.info("Testing get-checkpoints-summary to verify checkpoints exist")
        checkpoints_summary = self.get_checkpoints_summary()

        checkpoints_found = checkpoints_summary.get("checkpoints_found_in_db", 0)
        expected_checkpoints = checkpoints_summary.get("expected_checkpoints_count", 0)

        if checkpoints_found <= 0:
            self.error(f"Should have checkpoints, got: {checkpoints_found}")
            return False

        if checkpoints_found < expected_checkpoints:
            self.error(f"Should have {expected_checkpoints} checkpoints, got: {checkpoints_found}")
            return False

        self.info(
            f"Checkpoints validation passed - {checkpoints_found} checkpoints found "
            f"(expected: {expected_checkpoints})"
        )

        self.info("All syncinfo validation tests passed!")
        return True

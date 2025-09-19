import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import DbtoolMixin
from utils.dbtool import send_tx
from utils.utils import ProverClientSettings


@flexitest.register
class RevertCheckpointedBlockSeqTest(DbtoolMixin):
    """Test revert checkpointed block on sequencer"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq_waiter = self.create_strata_waiter(self.seqrpc)

        # Wait for genesis and generate some initial blocks
        seq_waiter.wait_until_genesis()

        # Generate some transactions to create blocks
        for _ in range(10):
            send_tx(self.w3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        seq_waiter.wait_until_epoch_finalized(1, timeout=30)

        # Generate more blocks to have a longer chain
        for _ in range(5):
            send_tx(self.w3)

        # Wait for both services to be in sync
        old_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        old_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        old_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(old_el_block_number), False)[
            "hash"
        ]
        self.info(f"OL block number: {old_ol_block_number}, EL block number: {old_el_block_number}")

        # Check if both services are at the same state before proceeding
        if old_ol_block_number != old_el_block_number:
            self.warning(
                f"OL and EL are not in sync: OL={old_ol_block_number}, EL={old_el_block_number}"
            )

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get checkpoints summary to find the latest checkpoint
        self.info("Getting checkpoints summary to find latest checkpoint")
        checkpoints_summary = self.get_checkpoints_summary()

        checkpoints_count = checkpoints_summary.get("checkpoints_found_in_db", 0)
        if checkpoints_count == 0:
            self.error("No checkpoints found")
            return False

        # Get the latest checkpoint index (checkpoints_count - 1)
        checkpt_idx_before_revert = checkpoints_count - 1
        self.info(f"Latest checkpoint index: {checkpt_idx_before_revert}")

        epoch_summary = self.get_epoch_summary(checkpt_idx_before_revert)
        self.info(f"Epoch summary before revert: {epoch_summary}")

        # Get the latest checkpoint details
        checkpt_before_revert = self.get_checkpoint(checkpt_idx_before_revert).get("checkpoint", {})

        # Extract the L2 range from the checkpoint
        batch_info = checkpt_before_revert.get("commitment", {}).get("batch_info", {})
        l2_range = batch_info.get("l2_range", {})

        if not l2_range:
            self.error("Could not find L2 range in checkpoint")
            return False

        # Get a block within the checkpointed range (use the first block in the range)
        checkpt_start_slot = l2_range[0].get("slot")
        checkpt_start_block_id = l2_range[0].get("blkid")
        target_slot = checkpt_start_slot
        target_block_id = checkpt_start_block_id

        if checkpt_start_slot is None or not checkpt_start_block_id:
            self.error("Could not find checkpoint start slot or block ID")
            return False

        self.info(
            f"Checkpoint start slot: {checkpt_start_slot}, block ID: {checkpt_start_block_id}"
        )

        # Try to revert to a checkpointed block with -c flag - this should succeed
        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-c")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info(f"revert-chainstate succeeded with return code {return_code}")
        self.info(f"Stdout: {stdout}")

        # Verify the chainstate was reverted correctly
        self.info("Verifying chainstate after revert")
        reverted_chainstate = self.get_chainstate(target_block_id)

        reverted_current_slot = reverted_chainstate.get("current_slot", 0)
        reverted_current_epoch = reverted_chainstate.get("current_epoch", 0)

        self.info(
            f"Reverted chainstate - current_slot: {reverted_current_slot}, "
            f"current_epoch: {reverted_current_epoch}"
        )

        # Verify that the chainstate was reverted to the target slot
        if reverted_current_slot != target_slot:
            self.error(
                f"Chainstate current_slot should be {target_slot} after revert, "
                f"got {reverted_current_slot}"
            )
            return False

        self.info("Chainstate revert verification passed")

        # Start services and verify they can continue from the reverted block
        self.reth.start()
        self.seq.start()
        self.seq_signer.start()

        # Wait for block production to resume to sync
        seq_waiter.wait_until_chain_tip_exceeds(old_ol_block_number + 1, timeout=120)

        # Wait for new epoch summary to be created
        self.info("Waiting for new epoch summary to be created after restart")
        epoch_summary = seq_waiter.wait_until_chain_epoch(
            checkpt_idx_before_revert + 1, timeout=120
        )
        self.info(f"Epoch summary after restart: {epoch_summary}")

        new_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        new_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        new_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(new_el_block_number), False)[
            "hash"
        ]

        self.info(
            f"old_ol_block_number: {old_ol_block_number}, "
            f"new_ol_block_number: {new_ol_block_number}"
        )
        self.info(f"chainstate reverted to target_slot: {target_slot}")
        self.info(f"old_el_blockhash: {old_el_blockhash}, new_el_blockhash: {new_el_blockhash}")

        assert old_el_blockhash != new_el_blockhash
        assert new_ol_block_number > old_ol_block_number
        assert new_el_block_number > old_el_block_number

        # Check if both services are at the same state after syncing
        if new_ol_block_number != new_el_block_number:
            self.warning(
                f"OL and EL are not in sync: OL={new_ol_block_number}, EL={new_el_block_number}"
            )

        self.info("Successfully reverted chainstate and resumed sync")
        return True

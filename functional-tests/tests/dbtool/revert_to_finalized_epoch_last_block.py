import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import DbtoolMixin
from utils.dbtool import send_tx
from utils.utils import ProverClientSettings


@flexitest.register
class RevertToFinalizedEpochLastBlockTest(DbtoolMixin):
    """Test that reverting to the last block of a finalized epoch works correctly"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Create waiters for better wait operations
        seq_waiter = self.create_strata_waiter(self.seqrpc)

        # Wait for genesis and generate some initial blocks
        seq_waiter.wait_until_genesis()

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(self.w3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        seq_waiter.wait_until_epoch_finalized(1, timeout=30)

        # Generate more blocks to have a longer chain beyond the finalized epoch
        for _ in range(10):
            send_tx(self.w3)

        # Wait for both services to be in sync
        ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {ol_block_number}, EL block number: {el_block_number}")
        old_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(el_block_number), False)["hash"]

        # Ensure both services are at the same state before proceeding
        if ol_block_number != el_block_number:
            self.error(f"Services are not in sync: OL={ol_block_number}, EL={el_block_number}")
            return False

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get sync information to find the finalized epoch last block
        self.info("Getting sync info to find finalized epoch last block")
        sync_info = self.get_syncinfo()

        finalized_epoch = sync_info.get("finalized_epoch")
        if not finalized_epoch:
            self.error("No finalized epoch found")
            return False

        target_slot = finalized_epoch.get("last_slot")
        target_block_id = finalized_epoch.get("last_blkid")

        if not target_block_id or target_slot is None:
            self.error("Could not find finalized epoch last block info")
            return False

        self.info(f"Finalized epoch last slot: {target_slot}, block ID: {target_block_id}")

        # Revert chainstate to the finalized epoch last block
        self.info(f"Testing revert-chainstate to finalized epoch last block {target_block_id}")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-u", "-d")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Chainstate revert to finalized epoch last block completed successfully")

        # Verify chainstate was reverted correctly
        self.info("Verifying chainstate after revert to finalized epoch last block")
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

        self.info("Chainstate revert to finalized epoch last block verification passed")

        # Start services and verify they can continue from the finalized epoch last block
        self.reth.start()
        self.seq.start()
        self.seq_signer.start()

        # Wait for block production to resume using waiter
        seq_waiter.wait_until_chain_tip_exceeds(target_slot)

        new_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        new_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        new_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(new_el_block_number), False)[
            "hash"
        ]

        self.info(f"After restart - OL: {new_ol_block_number}, EL: {new_el_block_number}")
        self.info(f"Block hash changed: {old_el_blockhash} -> {new_el_blockhash}")

        # Services should be in sync and continue processing from finalized epoch last block
        if new_ol_block_number != new_el_block_number:
            self.error(
                f"Services not in sync after restart: OL={new_ol_block_number}, "
                f"EL={new_el_block_number}"
            )
            return False

        self.info("Successfully reverted to finalized epoch last block and resumed processing")
        return True

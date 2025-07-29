import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import DbtoolMixin
from utils.dbtool import send_tx
from utils.utils import ProverClientSettings


@flexitest.register
class DbtoolRevertChainstateTest(DbtoolMixin):
    """Test that chainstate revert functionality works correctly with reth database sync"""

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

        # Generate more blocks to have a longer chain
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

        # Get sync information to find a revert target block id
        self.info("Getting sync info")
        sync_info = self.get_syncinfo()

        finalized_epoch = sync_info.get("finalized_epoch")
        finalized_epoch_last_slot = finalized_epoch.get("last_slot")
        previous_block = sync_info.get("previous_block")
        target_slot = previous_block.get("slot")
        target_block_id = previous_block.get("blkid")

        if target_slot == finalized_epoch_last_slot:
            self.info(
                f"Target slot {target_slot} is the same as finalized epoch last slot "
                f"{finalized_epoch_last_slot}"
            )

        if not target_block_id:
            self.error("Could not find previous block ID")
            return False

        # Revert chainstate to the target block
        self.info(f"Testing revert-chainstate to revert to block {target_block_id}")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id, "-u", "-d")

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Chainstate revert completed successfully")

        # Verify chainstate was reverted by checking again
        self.info("Testing get-chainstate after revert to verify changes")
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

        # Start both services together and let them sync
        self.reth.start()
        self.seq.start()
        self.seq_signer.start()

        # Wait for services to sync using waiter
        seq_waiter.wait_until_chain_tip_exceeds(target_slot + 1)

        new_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        new_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        new_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(new_el_block_number), False)[
            "hash"
        ]

        self.info(
            f"old_ol_block_number: {ol_block_number}, new_ol_block_number: {new_ol_block_number}"
        )
        self.info(f"chainstate reverted to target_slot: {target_slot}")
        self.info(f"old_el_blockhash: {old_el_blockhash}, new_el_blockhash: {new_el_blockhash}")

        assert old_el_blockhash != new_el_blockhash
        assert new_ol_block_number == new_el_block_number

        self.info("All chainstate revert tests passed!")
        return True

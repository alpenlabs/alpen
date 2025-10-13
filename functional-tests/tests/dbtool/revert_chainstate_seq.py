import flexitest

from envs import net_settings, testenv
from mixins.dbtool_mixin import SequencerDbtoolMixin
from utils.dbtool import (
    get_latest_checkpoint,
    restart_sequencer_after_revert,
    setup_revert_chainstate_test,
    target_end_of_epoch,
    verify_checkpoint_preserved,
    verify_revert_success,
)
from utils.utils import ProverClientSettings


@flexitest.register
class RevertChainstateSeqTest(SequencerDbtoolMixin):
    """Test revert chainstate on sequencer"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        # Setup: generate blocks and finalize epoch
        setup_revert_chainstate_test(self)

        # Capture state before revert
        old_ol_block_number = self.seqrpc.strata_syncStatus()["tip_height"]
        old_el_block_number = int(self.rethrpc.eth_blockNumber(), base=16)
        old_el_blockhash = self.rethrpc.eth_getBlockByNumber(hex(old_el_block_number), False)[
            "hash"
        ]
        self.info(f"OL block number: {old_ol_block_number}, EL block number: {old_el_block_number}")

        if old_ol_block_number != old_el_block_number:
            self.warning(
                f"OL and EL are not in sync: OL={old_ol_block_number}, EL={old_el_block_number}"
            )

        # Stop services to use dbtool
        self.seq_signer.stop()
        self.seq.stop()
        self.reth.stop()

        # Get checkpoint info and target block
        checkpt = get_latest_checkpoint(self)
        if not checkpt:
            return False

        target_block_id, target_slot = target_end_of_epoch(checkpt["l2_range"])

        # Ensure we have blocks outside checkpointed range
        sync_info = self.get_syncinfo()
        tip_slot = sync_info.get("l2_tip_height")

        if tip_slot and tip_slot <= target_slot:
            self.info("No blocks outside checkpointed range - test cannot proceed")
            return True

        self.info(f"Target slot: {target_slot}, target block ID: {target_block_id}")

        # Execute revert chainstate with no flags
        self.info(f"Testing revert-chainstate to {target_block_id} with no flags")
        return_code, stdout, stderr = self.revert_chainstate(target_block_id)

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Revert chainstate completed successfully")
        self.info(f"Stdout: {stdout}")

        # Verify chainstate and checkpoint data
        if not verify_revert_success(self, target_block_id, target_slot):
            return False

        if not verify_checkpoint_preserved(self, checkpt["idx"]):
            return False

        # Restart services and verify
        restart_sequencer_after_revert(self, target_slot, old_ol_block_number, checkpt["idx"])

        # Verify final state
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

        if new_ol_block_number != new_el_block_number:
            self.warning(
                f"OL and EL are not in sync: OL={new_ol_block_number}, EL={new_el_block_number}"
            )

        self.info("Successfully reverted sequencer chainstate and verified resync")
        return True

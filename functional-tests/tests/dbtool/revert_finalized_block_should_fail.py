import json
import os

import flexitest

from envs import net_settings, testenv
from utils import *
from utils.dbtool import (
    extract_json_from_output,
    run_dbtool_command,
    send_tx,
)


@flexitest.register
class RevertFinalizedBlockShouldFailTest(testenv.StrataTester):
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
        """Main test method"""
        seq = ctx.get_service("sequencer")
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        # rethrpc = reth.create_rpc()  # Unused variable

        # Wait for genesis and generate some initial blocks
        wait_for_genesis(seqrpc, timeout=20)

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(web3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        wait_until_epoch_finalized(seqrpc, 1, timeout=30)

        # Generate more blocks to have a longer chain
        for _ in range(10):
            send_tx(web3)

        # Stop services to ensure database is not being modified
        self.info("Stopping services to test dbtool")
        seq.stop()
        reth.stop()

        # Get the sequencer datadir from the test context
        seq_datadir = os.path.join(ctx.datadir_root, f"_{ctx.name}", "sequencer")
        self.info(f"Sequencer datadir: {seq_datadir}")

        # Verify the datadir exists
        if not os.path.exists(seq_datadir):
            self.error(f"Sequencer datadir does not exist: {seq_datadir}")
            return False

        self.info("Testing that reverting a finalized block fails")

        # Test 1: Get syncinfo to find finalized epoch
        self.info("Getting syncinfo to find finalized epoch")
        return_code, stdout, stderr = run_dbtool_command(seq_datadir, "get-syncinfo", "-o", "json")

        if return_code != 0:
            self.error(f"get-syncinfo failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            sync_info = json.loads(json_output)
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

            # Get the earliest block ID (which should be at slot 0)
            return_code, stdout, stderr = run_dbtool_command(
                seq_datadir, "get-l2-block", "-o", "json", finalized_epoch_last_blkid
            )

            if return_code != 0:
                self.error(f"get-l2-block failed with return code {return_code}")
                self.error(f"Stderr: {stderr}")
                return False

            try:
                json_output = extract_json_from_output(stdout)
                if not json_output:
                    self.error("No JSON found in get-l2-block output")
                    return False

                l2_block_info = json.loads(json_output)
                # First level header is a SignedL2BlockHeader, then L2BlockHeader
                l2_block_header = l2_block_info.get("header").get("header")
                target_block_id = l2_block_header.get("prev_block")
                target_slot = finalized_epoch_last_slot - 1

                if not target_block_id:
                    self.error("No previous block ID found in L2 block info")
                    return False

            except json.JSONDecodeError as e:
                self.error(f"Invalid JSON from get-l2-block: {e}")
                return False

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-syncinfo: {e}")
            return False

        # Test 2: Try to revert to `target_block_id`
        self.info(
            f"Attempting to revert to slot: {target_slot}, "
            f"block id: {target_block_id} (should fail)"
        )
        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "revert-chainstate", target_block_id
        )

        # The command should fail with an error
        if return_code == 0:
            self.error("revert-chainstate should have failed but succeeded")
            return False

        self.info("Reverting to a block inside finalized epoch fails as expected")
        return True

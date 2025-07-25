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
class DbtoolValidateSyncinfoTest(testenv.StrataTestBase):
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
        seq = ctx.get_service("sequencer")
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()

        reth_waiter = self.create_reth_waiter(rethrpc)
        seq_waiter = self.create_strata_waiter(seqrpc)

        # Wait for genesis and generate some initial blocks
        seq_waiter.wait_until_genesis()

        # Generate some transactions to create blocks
        for _ in range(5):
            send_tx(web3)

        # Wait for epoch finalization to ensure we have some finalized blocks
        seq_waiter.wait_until_epoch_finalized(0, timeout=30)

        # Generate more blocks to have a longer chain
        for _ in range(10):
            send_tx(web3)

        # Wait for more blocks to be generated
        reth_waiter.wait_until_eth_block_exceeds(10)

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

        # Test 1: Get sync info and validate L1/L2 chain positions
        self.info("Testing get-syncinfo to validate chain positions")
        return_code, stdout, stderr = run_dbtool_command(seq_datadir, "get-syncinfo", "-o", "json")

        if return_code != 0:
            self.error(f"get-syncinfo failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        # Parse and validate sync info
        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            sync_info = json.loads(json_output)

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

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-syncinfo: {e}")
            return False

        # Test 2: Verify L1 blocks exist
        self.info("Testing get-l1-summary to verify L1 blocks exist")
        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "get-l1-summary", "-o", "json"
        )

        if return_code != 0:
            self.error(f"get-l1-summary failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            l1_summary = json.loads(json_output)
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

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-l1-summary: {e}")
            return False

        # Test 3: Verify L2 blocks exist
        self.info("Testing get-l2-summary to verify L2 blocks exist")
        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "get-l2-summary", "-o", "json"
        )

        if return_code != 0:
            self.error(f"get-l2-block failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            l2_summary = json.loads(json_output)
            tip_slot = l2_summary.get("tip_slot", 0)
            all_blocks_present = l2_summary.get("all_blocks_present", False)

            if tip_slot <= 0:
                self.error(f"Should have L2 blocks, got tip_slot: {tip_slot}")
                return False

            if not all_blocks_present:
                self.error(f"All L2 blocks should be present, got: {all_blocks_present}")
                return False

            self.info(f"L2 blocks validation passed - tip slot: {tip_slot}, all blocks present")

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-l2-block: {e}")
            return False

        # Test 4: Verify checkpoints exist
        self.info("Testing get-checkpoints-summary to verify checkpoints exist")
        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "get-checkpoints-summary", "-o", "json"
        )

        if return_code != 0:
            self.error(f"get-checkpoints-summary failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            checkpoints_summary = json.loads(json_output)
            checkpoints_found = checkpoints_summary.get("checkpoints_found_in_db", 0)
            expected_checkpoints = checkpoints_summary.get("expected_checkpoints_count", 0)

            if checkpoints_found <= 0:
                self.error(f"Should have checkpoints, got: {checkpoints_found}")
                return False

            if checkpoints_found < expected_checkpoints:
                self.error(
                    f"Should have {expected_checkpoints} checkpoints, got: {checkpoints_found}"
                )
                return False

            self.info(
                f"Checkpoints validation passed - {checkpoints_found} checkpoints found "
                f"(expected: {expected_checkpoints})"
            )

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-checkpoints-summary: {e}")
            return False

        self.info("All syncinfo validation tests passed!")
        return True

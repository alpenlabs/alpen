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
class DbtoolRevertChainstateTest(testenv.StrataTester):
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
        seq = ctx.get_service("sequencer")
        seq_signer = ctx.get_service("sequencer_signer")
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()

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

        # Wait for both services to be in sync
        ol_block_number = seqrpc.strata_syncStatus()["tip_height"]
        el_block_number = int(rethrpc.eth_blockNumber(), base=16)
        self.info(f"OL block number: {ol_block_number}, EL block number: {el_block_number}")
        old_el_blockhash = rethrpc.eth_getBlockByNumber(hex(el_block_number), False)["hash"]
        # Ensure both services are at the same state before proceeding
        if ol_block_number != el_block_number:
            self.error(f"Services are not in sync: OL={ol_block_number}, EL={el_block_number}")
            return False

        print("stop sequencer")
        seq_signer.stop()
        print(f"stop seq @{ol_block_number}")
        seq.stop()
        reth.stop()

        # Get the sequencer datadir from the test context
        seq_datadir = os.path.join(ctx.datadir_root, f"_{ctx.name}", "sequencer")
        self.info(f"Sequencer datadir: {seq_datadir}")

        # Verify the datadir exists
        if not os.path.exists(seq_datadir):
            self.error(f"Sequencer datadir does not exist: {seq_datadir}")
            return False

        # 1: Get sync information to find a revert target block id
        self.info("Getting sync info")

        # Get syncinfo to understand current state
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
            finalized_epoch = sync_info.get("finalized_epoch")
            finalized_epoch_last_slot = finalized_epoch.get("last_slot")
            previous_block = sync_info.get("previous_block")
            target_slot = previous_block.get("slot")
            if not target_slot or target_slot == finalized_epoch_last_slot:
                self.info(
                    f"Target slot {target_slot} is the same as finalized epoch last slot "
                    f"{finalized_epoch_last_slot}"
                )

            target_block_id = previous_block.get("blkid")
            if not target_block_id:
                self.error("Could not find previous block ID")
                return False

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-syncinfo: {e}")
            return False

        # Test 2: Revert chainstate to the target block
        self.info(f"Testing revert-chainstate to revert to block {target_block_id}")

        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "revert-chainstate", target_block_id, "-u", "-d"
        )

        if return_code != 0:
            self.error(f"revert-chainstate failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        self.info("Chainstate revert completed successfully")

        # Test 4: Verify chainstate was reverted by checking again
        self.info("Testing get-chainstate after revert to verify changes")
        return_code, stdout, stderr = run_dbtool_command(
            seq_datadir, "get-chainstate", target_block_id, "-o", "json"
        )

        if return_code != 0:
            self.error(f"get-chainstate after revert failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            return False

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                return False

            reverted_chainstate = json.loads(json_output)
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

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from get-chainstate after revert: {e}")
            return False

        # Test 5: Start both services together and let them sync
        print("start reth")
        reth.start()
        print("start sequencer")
        seq.start()
        print("start sequencer signer")
        seq_signer.start()

        print("wait for block production to resume")
        wait_until(
            lambda: seqrpc.strata_syncStatus()["tip_height"] > target_slot + 1,
            error_with="not syncing blocks",
            timeout=30,
        )

        new_ol_block_number = seqrpc.strata_syncStatus()["tip_height"]
        new_el_block_number = int(rethrpc.eth_blockNumber(), base=16)
        new_el_blockhash = rethrpc.eth_getBlockByNumber(hex(new_el_block_number), False)["hash"]
        print(f"old_ol_block_number: {ol_block_number}")
        print(f"new_ol_block_number: {new_ol_block_number}")
        print(f"chainstate reverted to target_slot: {target_slot}")
        print(f"old_el_blockhash: {old_el_blockhash}")
        print(f"new_el_blockhash: {new_el_blockhash}")

        assert old_el_blockhash != new_el_blockhash
        assert new_ol_block_number == new_el_block_number

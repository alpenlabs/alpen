import json
import os
import subprocess

from web3 import Web3

from utils.utils import wait_for_genesis, wait_until_epoch_finalized


def send_tx(web3: Web3):
    """Send a simple transaction to generate activity"""
    dest = web3.to_checksum_address("deedf001900dca3ebeefdeadf001900dca3ebeef")
    txid = web3.eth.send_transaction(
        {
            "to": dest,
            "value": hex(1),
            "gas": hex(100000),
            "from": web3.address,
        }
    )
    print("txid", txid.to_0x_hex())
    web3.eth.wait_for_transaction_receipt(txid, timeout=5)


def run_dbtool_command(datadir: str, subcommand: str, *args) -> tuple[int, str, str]:
    """Run strata-dbtool command and return (return_code, stdout, stderr)"""
    cmd = ["strata-dbtool", "-d", datadir, subcommand] + list(args)
    print(f"Running command: {' '.join(cmd)}")

    result = subprocess.run(cmd, capture_output=True, text=True, cwd=os.path.dirname(datadir))

    return result.returncode, result.stdout, result.stderr


def extract_json_from_output(output: str) -> str:
    """Extract complete JSON objects from output, ignoring log lines"""
    # Find all potential JSON objects by looking for { } pairs
    start_idx = 0

    while True:
        start_idx = output.find("{", start_idx)
        if start_idx == -1:
            break

        # Count braces to find the complete JSON object
        brace_count = 0
        end_idx = -1

        for i in range(start_idx, len(output)):
            if output[i] == "{":
                brace_count += 1
            elif output[i] == "}":
                brace_count -= 1
                if brace_count == 0:
                    end_idx = i
                    break

        if end_idx != -1:
            json_str = output[start_idx : end_idx + 1]
            try:
                # Validate it's actually JSON
                json.loads(json_str)
                return json_str
            except json.JSONDecodeError:
                pass  # Not valid JSON, skip it

        start_idx = end_idx + 1 if end_idx != -1 else start_idx + 1

    return ""


def setup_revert_chainstate_test(
    test_instance,
    seqrpc_attr="seqrpc",
    web3_attr="web3",
    epoch_to_finalize=1,
    initial_txs=10,
    additional_txs=5,
):
    """
    Standard setup for revert chainstate tests:
     - wait for genesis
     - create initial transactions
     - finalize epoch
     - create additional transactions

    Args:
        test_instance: Test with dbtool mixin
        seqrpc_attr: attribute name for sequencer RPC, defaults to 'seqrpc'
        web3_attr: attribute name for web3 instance, defaults to 'web3'
        epoch_to_finalize: epoch to finalize, defaults to 1
        initial_txs: number of initial transactions to generate, defaults to 10
        additional_txs: number of additional transactions to generate, defaults to 5
    """
    seqrpc = getattr(test_instance, seqrpc_attr)
    web3 = getattr(test_instance, web3_attr)

    # Wait for genesis
    wait_for_genesis(seqrpc, timeout=20)

    # Generate initial transactions
    for _ in range(initial_txs):
        send_tx(web3)

    # Wait for epoch finalization
    wait_until_epoch_finalized(seqrpc, epoch_to_finalize, timeout=30)

    # Generate additional transactions
    for _ in range(additional_txs):
        send_tx(web3)


def get_latest_checkpoint(test_instance):
    """
    Get latest checkpoint info including L2 range.

    Returns:
        Dict with 'idx', 'checkpoint', 'l2_range' or None if no checkpoints
    """
    test_instance.info("Getting checkpoints summary to find latest checkpoint")
    summary = test_instance.get_checkpoints_summary()

    count = summary.get("checkpoints_found_in_db", 0)
    if count == 0:
        test_instance.error("No checkpoints found")
        return None

    idx = count - 1
    test_instance.info(f"Latest checkpoint index: {idx}")

    checkpoint = test_instance.get_checkpoint(idx).get("checkpoint", {})
    batch_info = checkpoint.get("commitment", {}).get("batch_info", {})
    l2_range = batch_info.get("l2_range", {})

    if not l2_range:
        test_instance.error("Could not find L2 range in checkpoint")
        return None

    return {"idx": idx, "checkpoint": checkpoint, "l2_range": l2_range}


def target_end_of_epoch(l2_range):
    """Get target block ID and slot at END of epoch (last block of checkpointed range)."""
    slot = l2_range[1].get("slot")
    block_id = l2_range[1].get("blkid")
    return block_id, slot


def target_start_of_epoch(l2_range):
    """Get target block ID and slot at START of epoch (first block of checkpointed range)."""
    slot = l2_range[0].get("slot")
    block_id = l2_range[0].get("blkid")
    return block_id, slot


def verify_revert_success(test_instance, target_block_id: str, expected_slot: int) -> bool:
    """Verify chainstate was reverted to expected slot."""
    test_instance.info("Verifying chainstate after revert")
    chainstate = test_instance.get_chainstate(target_block_id)

    actual_slot = chainstate.get("current_slot", 0)
    actual_epoch = chainstate.get("current_epoch", 0)

    test_instance.info(
        f"Reverted chainstate - current_slot: {actual_slot}, current_epoch: {actual_epoch}"
    )

    if actual_slot != expected_slot:
        test_instance.error(
            f"Chainstate current_slot should be {expected_slot} after revert, got {actual_slot}"
        )
        return False

    test_instance.info("Chainstate revert verification passed")
    return True


def verify_checkpoint_preserved(test_instance, checkpt_idx: int) -> bool:
    """Verify checkpoint data was preserved (for end-of-epoch revert)."""
    test_instance.info(
        "Validating that checkpoint data is preserved when reverting to end of epoch"
    )

    checkpt = test_instance.get_checkpoint(checkpt_idx)
    if not checkpt.get("checkpoint"):
        test_instance.error("Checkpoint was deleted when it should have been preserved")
        return False

    epoch_summary = test_instance.get_epoch_summary(checkpt_idx)
    if not epoch_summary.get("epoch_summary"):
        test_instance.error("Epoch summary was deleted when it should have been preserved")
        return False

    test_instance.info("Validation passed: Checkpoint and epoch summary preserved")
    return True


def verify_checkpoint_deleted(test_instance, checkpt_idx: int) -> bool:
    """Verify checkpoint data was deleted (for start-of-epoch revert)."""
    test_instance.info(
        "Validating that checkpoint data is deleted when reverting to beginning of epoch"
    )

    # Check that checkpoint was deleted
    try:
        checkpt_after_revert = test_instance.get_checkpoint(checkpt_idx)
        if checkpt_after_revert.get("checkpoint"):
            test_instance.error("Checkpoint was NOT deleted when it should have been deleted")
            return False
    except Exception:
        pass  # Expected - checkpoint should not exist

    # Check that epoch summary was deleted
    try:
        epoch_summary_after_revert = test_instance.get_epoch_summary(checkpt_idx)
        if epoch_summary_after_revert.get("epoch_summary"):
            test_instance.error("Epoch summary was NOT deleted when it should have been deleted")
            return False
    except Exception:
        pass  # Expected - epoch summary should not exist

    test_instance.info("Validation passed: Checkpoint and epoch summary deleted")
    return True


def restart_sequencer_after_revert(test_instance, target_slot: int, old_tip: int, checkpt_idx: int):
    """
    Restart sequencer services and wait for sync.

    This follows the critical service restart sequence:
    1. Start reth first and wait for it to sync
    2. Then start sequencer services
    3. Wait for block production to resume
    4. Wait for new epoch summary

    Args:
        test_instance: Test with dbtool mixin
        target_slot: The slot the chain was reverted to
        old_tip: The old tip before revert
        checkpt_idx: The checkpoint index before revert
    """
    from utils.utils import (
        wait_until_chain_epoch,
        wait_until_el_block_height,
        wait_until_l2_synced_to_height,
    )

    # Start reth first and wait for it to sync
    test_instance.info("Starting reth service...")
    test_instance.reth.start()

    test_instance.info("Waiting for reth to be ready and synced to reverted state...")
    target_block_number = target_slot
    wait_until_el_block_height(
        test_instance.rethrpc,
        target_block_number - 1,
        error_with="reth failed to sync after revert",
        timeout=120,
    )

    # Verify reth head
    try:
        current_block = test_instance.rethrpc.eth_getBlockByNumber(hex(target_block_number), False)
        if current_block and current_block.get("hash"):
            test_instance.info(f"Reth ready with head block: {current_block['hash']}")
        else:
            test_instance.warning("Could not verify reth head block hash")
    except Exception as e:
        test_instance.warning(f"Could not verify reth head block: {e}")

    # Start sequencer services
    test_instance.info("Starting sequencer services...")
    test_instance.seq.start()
    test_instance.seq_signer.start()

    # Wait for block production to resume
    wait_until_l2_synced_to_height(
        test_instance.seqrpc,
        old_tip + 1,
        error_with="expected blocks not produced after revert chainstate",
        timeout=30,
    )

    # Wait for new epoch summary to be created
    test_instance.info("Waiting for new epoch summary to be created after restart")
    wait_until_chain_epoch(
        test_instance.seqrpc,
        checkpt_idx + 1,
        error_with="new epoch summary not created after revert chainstate",
        timeout=120,
    )


def restart_fullnode_after_revert(
    test_instance, target_slot: int, old_seq_tip: int, checkpt_idx: int
):
    """
    Restart all services (sequencer + fullnode) after revert and wait for sync.

    Args:
        test_instance: Test with dbtool mixin
        target_slot: The slot the chain was reverted to
        old_seq_tip: The old sequencer tip before revert
        checkpt_idx: The checkpoint index before revert
    """
    from utils.utils import (
        wait_until_chain_epoch,
        wait_until_el_block_height,
        wait_until_l2_synced_to_height,
    )

    # Start reth first and wait for it to sync
    test_instance.info("Starting reth service...")
    test_instance.reth.start()

    test_instance.info("Waiting for reth to be ready and synced to reverted state...")
    target_block_number = target_slot
    wait_until_el_block_height(
        test_instance.rethrpc,
        target_block_number - 1,
        error_with="reth failed to sync after revert",
        timeout=120,
    )

    # Verify reth head
    try:
        current_block = test_instance.rethrpc.eth_getBlockByNumber(hex(target_block_number), False)
        if current_block and current_block.get("hash"):
            test_instance.info(f"Reth ready with head block: {current_block['hash']}")
        else:
            test_instance.warning("Could not verify reth head block hash")
    except Exception as e:
        test_instance.warning(f"Could not verify reth head block: {e}")

    # Start sequencer services
    test_instance.info("Starting sequencer services...")
    test_instance.seq.start()
    test_instance.seq_signer.start()

    # Start fullnode services
    test_instance.info("Starting fullnode services...")
    test_instance.follower_1_reth.start()
    test_instance.follower_1_node.start()

    # Wait for block production to resume on sequencer
    wait_until_l2_synced_to_height(
        test_instance.seqrpc,
        old_seq_tip + 1,
        error_with="expected blocks not produced after revert chainstate",
        timeout=30,
    )

    # Wait for fullnode to catch up
    wait_until_l2_synced_to_height(
        test_instance.follower_1_rpc,
        old_seq_tip + 1,
        error_with="fullnode did not catch up to sequencer",
        timeout=120,
    )

    # Wait for new epoch summary to be created
    test_instance.info("Waiting for new epoch summary to be created after restart")
    wait_until_chain_epoch(
        test_instance.follower_1_rpc,
        checkpt_idx + 1,
        error_with="new epoch summary not created after revert chainstate",
        timeout=120,
    )

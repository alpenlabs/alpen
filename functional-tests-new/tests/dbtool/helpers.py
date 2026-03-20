"""Helpers for invoking strata-dbtool in functional-tests-new."""

import json
import logging
import subprocess
from pathlib import Path
from typing import Any

from bitcoinlib.services.bitcoind import BitcoindClient

from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


def run_dbtool(datadir: str, *args: str, timeout: int = 60) -> tuple[int, str, str]:
    """Run strata-dbtool against a datadir and return (code, stdout, stderr)."""
    cmd = ["strata-dbtool", "-d", datadir, *args]
    logger.info("Running command: %s", " ".join(cmd))
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=str(Path(datadir).parent),
        timeout=timeout,
    )
    if result.returncode == 0:
        if result.stdout:
            logger.info("Stdout: %s", result.stdout.strip())
    else:
        if result.stderr:
            logger.info("Stderr: %s", result.stderr.strip())
    return result.returncode, result.stdout, result.stderr


def extract_json_from_output(output: str) -> dict[str, Any]:
    """Extract and decode first valid JSON object from output text."""
    start = 0
    while True:
        start = output.find("{", start)
        if start == -1:
            raise ValueError(f"No JSON object found in output: {output}")

        depth = 0
        end = -1
        for idx in range(start, len(output)):
            if output[idx] == "{":
                depth += 1
            elif output[idx] == "}":
                depth -= 1
                if depth == 0:
                    end = idx
                    break

        if end == -1:
            raise ValueError(f"Unterminated JSON object in output: {output}")

        candidate = output[start : end + 1]
        try:
            return json.loads(candidate)
        except json.JSONDecodeError:
            start = end + 1
            continue


def run_dbtool_json(datadir: str, *args: str, timeout: int = 60) -> dict[str, Any]:
    """Run strata-dbtool with JSON output and parse response."""
    code, stdout, stderr = run_dbtool(datadir, *args, "-o", "json", timeout=timeout)
    if code != 0:
        raise RuntimeError(f"strata-dbtool command failed ({' '.join(args)}): {stderr or stdout}")
    return extract_json_from_output(stdout)


def load_rollup_genesis_height(datadir: str) -> int:
    """Load genesis L1 height from rollup-params.json in the node datadir."""
    rollup_path = Path(datadir) / "rollup-params.json"
    with open(rollup_path) as f:
        params = json.load(f)
    return int(params["genesis_l1_view"]["blk"]["height"])


def ol_genesis_slot() -> int:
    """Return the OL genesis slot."""
    return 0


def _target_start_of_epoch_from_tip(
    datadir: str, tip_block_id: str, prev_terminal_slot: int, slots_per_epoch: int
) -> tuple[str, int]:
    """
    Return (block_id, slot) for the first OL block in the target epoch.

    This mirrors legacy test intent ("start of epoch") while using OL dbtool outputs
    where checkpoint details expose tip commitment and epoch summary exposes prev terminal.
    """
    target_slot = prev_terminal_slot + 1
    tip_block = run_dbtool_json(datadir, "get-ol-block", tip_block_id)
    tip_slot = int(tip_block["header_slot"])
    if tip_slot < target_slot:
        raise AssertionError(
            f"Tip slot {tip_slot} is before target slot {target_slot} when locating epoch start"
        )

    if slots_per_epoch <= 0:
        raise AssertionError(f"Invalid slots_per_epoch: {slots_per_epoch}")
    max_steps = max((tip_slot - target_slot) + 2, slots_per_epoch + 2)
    cur = tip_block_id
    for _ in range(max_steps):
        block = run_dbtool_json(datadir, "get-ol-block", cur)
        slot = int(block["header_slot"])
        if slot == target_slot:
            return cur, target_slot
        if slot < target_slot:
            break
        cur = block["header_prev_blkid"]
    raise AssertionError(
        f"Failed to find epoch start block at slot {target_slot} within "
        f"{max_steps} steps (tip_slot={tip_slot}, slots_per_epoch={slots_per_epoch})"
    )


def wait_for_finalized_epoch_with_mining(
    strata_service: Any,
    strata_rpc: Any,
    btc_rpc: BitcoindClient,
    mine_address: str,
    target_epoch: int = 1,
    timeout: int = 120,
    step: float = 1.0,
) -> dict[str, Any]:
    """
    Mine L1 blocks until finalized epoch reaches target.
    """
    return wait_until_with_value(
        lambda: (
            btc_rpc.proxy.generatetoaddress(1, mine_address),
            strata_service.get_sync_status(strata_rpc).get("finalized"),
        )[1],
        lambda v: (
            isinstance(v, dict)
            and v.get("epoch", -1) >= target_epoch
            and v.get("last_blkid") != "00" * 32
        ),
        error_with=f"Timed out waiting for finalized epoch >= {target_epoch}",
        timeout=timeout,
        step=step,
    )


def wait_for_completed_epoch(
    strata_service: Any,
    strata_rpc: Any,
    target_epoch: int,
    *,
    timeout: int = 120,
    error_with: str | None = None,
) -> dict[str, Any]:
    """Wait until target epoch is completed (tip.epoch > target_epoch)."""
    return wait_until_with_value(
        lambda: strata_service.get_sync_status(strata_rpc).get("tip"),
        lambda tip: isinstance(tip, dict)
        and isinstance(tip.get("epoch"), int)
        and tip["epoch"] > target_epoch,
        timeout=timeout,
        error_with=error_with or f"Timed out waiting for epoch {target_epoch} to complete",
    )


def target_start_of_checkpointed_epoch(
    datadir: str,
    checkpoint: dict[str, Any],
    slots_per_epoch: int,
) -> tuple[str, int]:
    """Get target block ID/slot at start of checkpointed epoch."""
    checkpoint_epoch = checkpoint.get("checkpoint_epoch")
    if not isinstance(checkpoint_epoch, int):
        raise AssertionError(
            "Invalid get-checkpoint JSON: missing/invalid checkpoint_epoch "
            f"(got {checkpoint_epoch!r})"
        )
    epoch_summary = run_dbtool_json(datadir, "get-epoch-summary", str(checkpoint_epoch))
    prev_terminal_slot = epoch_summary["epoch_summary"]["prev_terminal"]["slot"]
    tip_blkid, _ = parse_checkpoint_tip_block_and_slot(checkpoint)
    return _target_start_of_epoch_from_tip(
        datadir,
        tip_blkid,
        prev_terminal_slot,
        slots_per_epoch,
    )


def parse_finalized_epoch_from_syncinfo(syncinfo: dict[str, Any]) -> tuple[str, int]:
    """
    Parse and validate finalized epoch fields from get-syncinfo JSON.

    Returns:
        Tuple of `(last_blkid, last_slot)`.
    """
    finalized_epoch = syncinfo.get("finalized_epoch")
    if not isinstance(finalized_epoch, dict):
        raise AssertionError(
            f"Invalid get-syncinfo JSON: missing/invalid 'finalized_epoch' object. "
            f"Top-level keys={sorted(syncinfo.keys())}"
        )

    last_blkid = finalized_epoch.get("last_blkid")
    last_slot = finalized_epoch.get("last_slot")
    if not isinstance(last_blkid, str):
        raise AssertionError(
            "Invalid get-syncinfo JSON: finalized_epoch.last_blkid missing or not a string"
        )
    try:
        parsed_last_slot = int(last_slot)
    except (TypeError, ValueError) as exc:
        raise AssertionError(
            "Invalid get-syncinfo JSON: finalized_epoch.last_slot missing or not an integer"
        ) from exc

    return last_blkid, parsed_last_slot


def parse_ol_block_parent_blkid(ol_block: dict[str, Any]) -> str:
    """Parse and validate parent block id from get-ol-block JSON."""
    parent_blkid = ol_block.get("header_prev_blkid")
    if not isinstance(parent_blkid, str):
        raise AssertionError(
            f"Invalid get-ol-block JSON: missing/invalid 'header_prev_blkid'. "
            f"Available keys={sorted(ol_block.keys())}"
        )
    return parent_blkid


def parse_ol_block_slot(ol_block: dict[str, Any]) -> int:
    """Parse and validate slot from get-ol-block JSON."""
    slot = ol_block.get("header_slot")
    try:
        parsed_slot = int(slot)
    except (TypeError, ValueError) as exc:
        raise AssertionError(
            "Invalid get-ol-block JSON: missing/invalid 'header_slot' (expected integer)"
        ) from exc
    return parsed_slot


def revert_ol_state(
    datadir: str,
    target_block_id: str,
    *,
    force: bool = True,
    delete_blocks: bool = False,
    revert_checkpointed: bool = False,
    timeout: int = 60,
) -> tuple[int, str, str]:
    """Execute revert-ol-state with optional flags."""
    args = ["revert-ol-state", target_block_id]
    if force:
        args.append("-f")
    if delete_blocks:
        args.append("-d")
    if revert_checkpointed:
        args.append("-c")
    return run_dbtool(datadir, *args, timeout=timeout)


def assert_checkpoint_present(datadir: str, epoch: int) -> None:
    """Assert checkpoint entry is present for epoch."""
    checkpoint = run_dbtool_json(datadir, "get-checkpoint", str(epoch))
    assert checkpoint.get("checkpoint_epoch") is not None


def assert_epoch_summary_present(datadir: str, epoch: int) -> None:
    """Assert epoch summary entry is present for epoch."""
    epoch_summary = run_dbtool_json(datadir, "get-epoch-summary", str(epoch))
    assert epoch_summary.get("epoch_summary") is not None


def assert_checkpoint_deleted(datadir: str, epoch: int) -> None:
    """Assert checkpoint entry is deleted for epoch."""
    code, _, _ = run_dbtool(datadir, "get-checkpoint", str(epoch), "-o", "json")
    assert code != 0


def assert_epoch_summary_deleted(datadir: str, epoch: int) -> None:
    """Assert epoch summary entry is deleted for epoch."""
    code, _, _ = run_dbtool(datadir, "get-epoch-summary", str(epoch), "-o", "json")
    assert code != 0


def wait_for_tip_exceeds(
    strata_service: Any,
    strata_rpc: Any,
    old_tip: int,
    *,
    timeout: int = 60,
    error_with: str,
) -> int:
    """Wait until node tip exceeds old tip and return new tip."""
    return wait_until_with_value(
        lambda: strata_service.get_cur_block_height(strata_rpc),
        lambda h: h > old_tip,
        timeout=timeout,
        error_with=error_with,
    )


def wait_for_seq_fn_progress(
    seq_service: Any,
    fn_service: Any,
    seq_rpc: Any,
    fn_rpc: Any,
    *,
    additional_blocks: int = 4,
    timeout_per_block: int = 10,
    fn_sync_timeout: int = 60,
) -> tuple[int, int]:
    """
    Wait for sequencer to produce additional blocks and fullnode to catch up.

    Returns:
        Tuple of `(sequencer_tip, fullnode_tip)` after progress/sync.
    """
    seq_service.wait_for_additional_blocks(
        additional_blocks,
        seq_rpc,
        timeout_per_block=timeout_per_block,
    )
    seq_tip = seq_service.get_cur_block_height(seq_rpc)
    fn_service.wait_for_block_height(seq_tip, rpc=fn_rpc, timeout=fn_sync_timeout)
    fn_tip = fn_service.get_cur_block_height(fn_rpc)
    return seq_tip, fn_tip


def setup_revert_ol_state_test(
    strata_service: Any,
    btc_service: Any,
    *,
    target_epoch: int = 1,
    additional_blocks: int = 10,
    rpc_timeout: int = 20,
    timeout_per_block: int = 10,
    finalization_timeout: int = 120,
) -> dict[str, Any]:
    """Prepare sequencer+bitcoin state and return ready RPC handles.

    Steps:
    1. Wait for sequencer RPC readiness.
    2. Wait for additional OL blocks.
    3. Mine L1 until `target_epoch` is finalized.

    Returns:
        Dict containing:
        - `rpc`: sequencer RPC handle.
        - `btc_rpc`: bitcoin RPC handle.
    """
    strata_rpc = strata_service.wait_for_rpc_ready(timeout=rpc_timeout)
    btc_rpc = btc_service.create_rpc()
    mine_address = btc_rpc.proxy.getnewaddress()

    strata_service.wait_for_additional_blocks(
        additional_blocks, strata_rpc, timeout_per_block=timeout_per_block
    )
    wait_for_finalized_epoch_with_mining(
        strata_service,
        strata_rpc,
        btc_rpc,
        mine_address,
        target_epoch=target_epoch,
        timeout=finalization_timeout,
    )
    return {
        "rpc": strata_rpc,
        "btc_rpc": btc_rpc,
    }


def setup_revert_ol_state_test_fullnode(
    seq_service: Any,
    fn_service: Any,
    btc_service: Any,
    *,
    target_epoch: int = 1,
    additional_blocks: int = 10,
    rpc_timeout: int = 20,
    timeout_per_block: int = 10,
    finalization_timeout: int = 120,
    fn_min_sync_delta: int = 6,
    fn_sync_timeout: int = 60,
) -> dict[str, Any]:
    """Prepare sequencer+fullnode+bitcoin state and return ready RPC handles.

    Steps:
    1. Wait for sequencer RPC readiness.
    2. Wait for additional OL blocks.
    3. Mine L1 until `target_epoch` is finalized.
    4. Wait for fullnode RPC readiness and catch-up to sequencer tip.

    Returns:
        Dict containing:
        - `seq_rpc`: sequencer RPC handle.
        - `fn_rpc`: fullnode RPC handle.
        - `btc_rpc`: bitcoin RPC handle.
    """
    seq_rpc = seq_service.wait_for_rpc_ready(timeout=rpc_timeout)
    btc_rpc = btc_service.create_rpc()
    mine_address = btc_rpc.proxy.getnewaddress()
    seq_service.wait_for_additional_blocks(
        additional_blocks, seq_rpc, timeout_per_block=timeout_per_block
    )
    wait_for_finalized_epoch_with_mining(
        seq_service,
        seq_rpc,
        btc_rpc,
        mine_address,
        target_epoch=target_epoch,
        timeout=finalization_timeout,
    )

    fn_rpc = fn_service.wait_for_rpc_ready(timeout=rpc_timeout)
    seq_tip_slot = seq_service.get_cur_block_height(seq_rpc)
    fn_service.wait_for_block_height(seq_tip_slot, rpc=fn_rpc, timeout=fn_sync_timeout)

    return {
        "seq_rpc": seq_rpc,
        "fn_rpc": fn_rpc,
        "btc_rpc": btc_rpc,
    }


def get_latest_checkpoint(datadir: str) -> dict[str, Any]:
    """Fetch latest checkpoint."""
    genesis_height = load_rollup_genesis_height(datadir)
    checkpoints_summary = run_dbtool_json(datadir, "get-checkpoints-summary", str(genesis_height))
    checkpoints_found = int(checkpoints_summary["checkpoints_found_in_db"])
    expected_checkpoints = checkpoints_summary.get("expected_checkpoints_count")
    if isinstance(expected_checkpoints, int) and int(expected_checkpoints) != checkpoints_found:
        logger.info(
            "Checkpoint count mismatch: expected_checkpoints_count=%s checkpoints_found_in_db=%s",
            expected_checkpoints,
            checkpoints_found,
        )
    if checkpoints_found == 0:
        raise AssertionError("No checkpoints found in db")

    # In dbtool tests we assume checkpoint epochs are 1-based and contiguous.
    # Under that invariant, count of checkpoints in DB equals latest checkpoint epoch.
    latest_checkpoint_epoch = checkpoints_found

    return run_dbtool_json(datadir, "get-checkpoint", str(latest_checkpoint_epoch))


def target_end_of_checkpointed_epoch(checkpoint: dict[str, Any]) -> tuple[str, int]:
    """Get target block ID/slot at end of checkpointed epoch."""
    return parse_checkpoint_tip_block_and_slot(checkpoint)


def parse_checkpoint_tip_block_and_slot(checkpoint: dict[str, Any]) -> tuple[str, int]:
    """Parse checkpoint tip block id/slot from get-checkpoint output."""
    tip_blkid = checkpoint.get("tip_ol_blkid")
    tip_slot = checkpoint.get("tip_ol_slot")
    if not isinstance(tip_blkid, str):
        raise AssertionError(
            f"Invalid get-checkpoint JSON: missing/invalid tip block id. "
            f"Available keys={sorted(checkpoint.keys())}"
        )
    try:
        parsed_slot = int(tip_slot)
    except (TypeError, ValueError) as exc:
        raise AssertionError(
            "Invalid get-checkpoint JSON: missing/invalid tip slot (expected tip_ol_slot)"
        ) from exc
    return tip_blkid, parsed_slot


def verify_checkpoint_preserved(datadir: str, epoch: int) -> bool:
    """Verify checkpoint and epoch summary are present for epoch."""
    try:
        assert_checkpoint_present(datadir, epoch)
        assert_epoch_summary_present(datadir, epoch)
    except Exception:
        return False
    return True


def verify_checkpoint_deleted(datadir: str, epoch: int) -> bool:
    """Verify checkpoint and epoch summary are deleted for epoch."""
    try:
        assert_checkpoint_deleted(datadir, epoch)
        assert_epoch_summary_deleted(datadir, epoch)
    except Exception:
        return False
    return True


def restart_sequencer_after_revert(
    strata_service: Any,
    old_tip: int,
    *,
    target_epoch: int | None = None,
    rpc_timeout: int = 30,
    wait_timeout: int = 60,
    epoch_wait_timeout: int = 120,
    error_with: str = "Sequencer did not resume after OL state revert",
) -> tuple[Any, int]:
    """Restart sequencer and wait for tip progression."""
    strata_service.start()
    rpc = strata_service.wait_for_rpc_ready(timeout=rpc_timeout)
    new_tip = wait_for_tip_exceeds(
        strata_service,
        rpc,
        old_tip,
        timeout=wait_timeout,
        error_with=error_with,
    )
    if target_epoch is not None:
        wait_for_completed_epoch(
            strata_service,
            rpc,
            target_epoch,
            timeout=epoch_wait_timeout,
            error_with=(
                f"Sequencer did not reach expected post-restart epoch (target_epoch={target_epoch})"
            ),
        )
    return rpc, new_tip


def verify_tip_resumed_with_new_blkid(
    strata_service: Any,
    rpc: Any,
    old_tip_slot: int,
    old_tip_blkid: str,
    resumed_tip: int,
) -> dict[str, Any]:
    """Verify resumed tip moved forward and tip block id changed from pre-revert value."""
    resumed_sync = strata_service.get_sync_status(rpc)
    tip = resumed_sync["tip"]
    assert resumed_tip > old_tip_slot
    assert tip["blkid"] != old_tip_blkid
    return resumed_sync


def restart_fullnode_after_revert(
    seq_service: Any,
    fn_service: Any,
    old_seq_tip: int,
    old_fn_tip: int,
    *,
    target_epoch: int | None = None,
    rpc_timeout: int = 30,
    wait_timeout: int = 60,
    epoch_wait_timeout: int = 120,
    seq_error_with: str = "Sequencer did not resume after fullnode OL state revert",
    fn_error_with: str = "Fullnode did not resume after OL state revert",
) -> tuple[Any, Any, int, int]:
    """Restart sequencer/fullnode and wait for both tips to progress."""
    seq_service.start()
    seq_rpc = seq_service.wait_for_rpc_ready(timeout=rpc_timeout)
    fn_service.start()
    fn_rpc = fn_service.wait_for_rpc_ready(timeout=rpc_timeout)

    new_seq_tip = wait_for_tip_exceeds(
        seq_service,
        seq_rpc,
        old_seq_tip,
        timeout=wait_timeout,
        error_with=seq_error_with,
    )
    fullnode_sync_target = max(old_seq_tip, old_fn_tip)
    new_fn_tip = wait_for_tip_exceeds(
        fn_service,
        fn_rpc,
        fullnode_sync_target,
        timeout=wait_timeout,
        error_with=fn_error_with,
    )
    if target_epoch is not None:
        wait_for_completed_epoch(
            fn_service,
            fn_rpc,
            target_epoch,
            timeout=epoch_wait_timeout,
            error_with=(
                f"Fullnode did not reach expected post-restart epoch (target_epoch={target_epoch})"
            ),
        )
    return seq_rpc, fn_rpc, new_seq_tip, new_fn_tip

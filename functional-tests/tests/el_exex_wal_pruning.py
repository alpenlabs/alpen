import glob
import logging
import os

import flexitest
from web3 import Web3

from envs import net_settings, testenv
from utils import ProverClientSettings, wait_for_genesis, wait_until, wait_until_epoch_finalized


def send_tx(web3: Web3):
    dest = web3.to_checksum_address("deedf001900dca3ebeefdeadf001900dca3ebeef")
    txid = web3.eth.send_transaction(
        {
            "to": dest,
            "value": hex(1),
            "gas": hex(21000),
            "from": web3.address,
        }
    )
    web3.eth.wait_for_transaction_receipt(txid, timeout=30)


@flexitest.register
class ElExexWalPruningTest(testenv.StrataTester):
    """
    Verifies that ExEx WAL files are pruned after epoch finalization.

    The ProverWitnessGenerator ExEx emits FinishedHeight after processing each block.
    Once an epoch is finalized (via forkchoiceUpdated with finalizedBlockHash), reth
    should call finalize_wal() and delete old WAL files.

    This test catches a regression where FinishedHeight reported block number 0
    instead of the actual block number, causing WAL files to never be pruned.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seqrpc = ctx.get_service("sequencer").create_rpc()
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()

        wait_for_genesis(seqrpc, timeout=20)

        # WAL directory is under reth's datadir
        wal_dir = os.path.join(reth.datadir, "exex", "wal")

        # Wait for some blocks to be produced so WAL files accumulate
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 10,
            error_with="blocks not advancing",
            timeout=30,
        )

        # Record WAL files before finalization
        wal_files_before = set(glob.glob(os.path.join(wal_dir, "*.wal")))
        logging.info(f"WAL files before finalization: {len(wal_files_before)}")
        assert wal_files_before, "Expected WAL files to exist before finalization"

        # Send some transactions to ensure non-empty blocks (workaround for reth restart issues)
        web3 = reth.create_web3()
        for _ in range(3):
            send_tx(web3)

        # Epoch 1 finalization triggers forkchoiceUpdated with finalizedBlockHash,
        # which triggers finalize_wal() in the ExEx manager.
        wait_until_epoch_finalized(seqrpc, 1, timeout=120)
        logging.info("Epoch 1 finalized")

        # Wait for reth to process the finalization and prune WAL
        def wal_files_pruned():
            current = set(glob.glob(os.path.join(wal_dir, "*.wal")))
            return len(wal_files_before - current) > 0

        wait_until(
            wal_files_pruned,
            error_with="No WAL files were pruned after finalization. "
            "FinishedHeight may be reporting incorrect block numbers.",
            timeout=30,
        )

        # Verify WAL pruning by tracking specific files by their IDs.
        wal_files_after = set(glob.glob(os.path.join(wal_dir, "*.wal")))
        pruned_files = wal_files_before - wal_files_after
        remaining_original = wal_files_before & wal_files_after

        def wal_file_id(path):
            return int(os.path.basename(path).removesuffix(".wal"))

        pruned_ids = sorted(wal_file_id(f) for f in pruned_files)
        remaining_ids = sorted(wal_file_id(f) for f in remaining_original)

        logging.info(
            f"WAL files: {len(wal_files_before)} before, {len(wal_files_after)} after, "
            f"pruned IDs: {pruned_ids}, remaining original IDs: {remaining_ids}"
        )

        # Pruned IDs must all be lower than surviving IDs.
        if remaining_ids:
            assert max(pruned_ids) < min(remaining_ids), (
                f"Pruning did not remove the oldest files first. "
                f"Pruned IDs: {pruned_ids}, Remaining original IDs: {remaining_ids}"
            )

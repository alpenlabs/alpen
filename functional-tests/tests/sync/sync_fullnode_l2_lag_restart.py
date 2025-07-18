import logging

import flexitest

from envs import testenv
from utils import *

FOLLOW_DIST = 1


@flexitest.register
class SyncFullNodeL2LagRestartTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        env = testenv.HubNetworkEnvConfig(
            110, rollup_settings=RollupParamsSettings.new_default().fast_batch()
        )
        ctx.set_env(env)

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("seq_node")
        seqrpc = seq.create_rpc()
        fullnode = ctx.get_service("follower_1_node")
        fnrpc = fullnode.create_rpc()
        seq_waiter = self.create_strata_waiter(seqrpc, timeout=60)
        fn_waiter = self.create_strata_waiter(fnrpc, timeout=60)

        # Pick a recent slot and make sure they're both the same.
        seqss = seqrpc.strata_syncStatus()

        # Now pause the sync worker so that we can have finalized epoch on L1,
        # but not corresponding block on L2 in full node
        paused = fnrpc.debug_pause_resume("SyncWorker", "PauseUntilResume")
        assert paused, "Should pause the fullnode sync worker"

        cur_epoch = seqss["cur_epoch"]
        self.info(f"{cur_epoch=}")

        # wait for fn to sync up to end of current sequencer epoch
        # L1 reader and csm should still be running and syncing with L2 sync paused/
        fn_waiter.wait_until_epoch_confirmed(cur_epoch)

        # Wait until some more epochs are finalized in sequencer so we have plenty of blocks
        # to sync up when we resume fn
        seq_waiter.wait_until_epoch_finalized(cur_epoch + 3)

        # Full node tip after sync is paused
        fn_ss = fnrpc.strata_syncStatus()
        fn_tip = fn_ss["tip_height"]

        # Get corresponding checkpoint block
        checkpt_info = seqrpc.strata_getCheckpointInfo(cur_epoch + 3)
        checkpt_l1_blk_height = checkpt_info["l1_reference"]["block_height"]

        # FN tip after fn catches upto the buried checkpoint, should be the same as before
        new_fn_tip = fnrpc.strata_syncStatus()["tip_height"]
        assert fn_tip == new_fn_tip, "Fn tip should not progress while syncing is paused"
        seq_tip = seqrpc.strata_syncStatus()["tip_height"]
        assert new_fn_tip < seq_tip, "Fn tip should be less than sequencer tip"

        # stop and restart fullnode

        fullnode.stop()

        fullnode.start()

        # Now check the epoch finalization, it should finalize since full node has resumed l2 sync
        wait_until_with_value(
            lambda: (
                fnrpc.strata_clientStatus()["tip_l1_block"]["height"],
                seqrpc.strata_clientStatus()["tip_l1_block"]["height"],
            ),
            lambda v: v[0] >= checkpt_l1_blk_height,
            error_with="Fullnode L1 sync did not catch upto buried checkpoint",
            timeout=60,
            debug=True,
        )

        # Let's check that eventually the fullnode syncs with sequencer
        wait_until(
            fn_syncs_with_seq(fnrpc, seqrpc),
            error_with="Full node could not sync with sequencer",
            timeout=60,
        )


def fn_syncs_with_seq(fnrpc, seqrpc):
    def _f():
        fnss = fnrpc.strata_syncStatus()
        seqss = seqrpc.strata_syncStatus()
        seq_tip_slot = seqss["tip_height"]
        fn_tip_slot = fnss["tip_height"]

        logging.info(f"Seq tip slot {seq_tip_slot}, fn tip slot {fn_tip_slot}")
        return fn_tip_slot == seq_tip_slot

    return _f


def check_both_at_same_slot(seqrpc, fnrpc, check_slot):
    seq_headers = seqrpc.strata_getHeadersAtIdx(check_slot)
    assert len(seq_headers) > 0, f"seq node missing headers at slot {check_slot}"

    fn_headers = fnrpc.strata_getHeadersAtIdx(check_slot)
    assert len(fn_headers) > 0, f"follower node missing headers at slot {check_slot}"

    seq_hdr = seq_headers[0]
    fn_hdr = fn_headers[0]
    assert seq_hdr == fn_hdr, f"headers mismatched at slot {check_slot}"

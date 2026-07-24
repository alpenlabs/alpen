"""Verify checkpoint publication remains exactly once across sequencer restart.

The native prover produces deterministic checkpoint proofs, so the pre-fix
writer deduplication can make this test pass even without the STR-4015 repair.
Failing-first unit tests cover the randomized-proof regression. This test
provides end-to-end exactly-once and post-restart liveness coverage.
"""

import logging
import time
from collections import defaultdict

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import (
    CHECKPOINT_SUBPROTOCOL_ID,
    OL_STF_CHECKPOINT_TX_TYPE,
    extract_posted_checkpoint_payload,
    mine_until_finalized_epoch,
    parse_checkpoint_epoch,
    parse_checkpoint_payload,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointNoDoubleSubmissionOnRestart(StrataNodeTest):
    """A queued checkpoint backlog survives restart without duplicate L1 posts."""

    BACKLOG_LAST_EPOCH = 4
    RESTART_PAUSE_SECONDS = 2

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                l1_reorg_safe_depth=3,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata: StrataService = self.get_service(ServiceType.Strata)
        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        admin_rpc = strata.create_admin_rpc()
        mine_addr = btc_rpc.proxy.getnewaddress()
        first_scan_height = btc_rpc.proxy.getblockcount() + 1

        self._wait_for_enqueued_backlog(
            strata_rpc,
            admin_rpc,
            first_epoch=1,
            last_epoch=1,
        )
        self._wait_for_checkpoint_in_mempool(
            btc_rpc,
            epoch=1,
        )
        btc_rpc.proxy.generatetoaddress(1, mine_addr)
        self._wait_for_epoch_one_seen(strata_rpc)
        self._wait_for_enqueued_backlog(
            strata_rpc,
            admin_rpc,
            first_epoch=2,
            last_epoch=self.BACKLOG_LAST_EPOCH,
        )

        logger.info(
            "restarting sequencer with checkpoint epochs 2..%s queued",
            self.BACKLOG_LAST_EPOCH,
        )
        strata.stop()
        time.sleep(self.RESTART_PAUSE_SECONDS)
        strata.start()
        strata_rpc = strata.wait_for_rpc_ready(timeout=30)

        fresh_epoch = self.BACKLOG_LAST_EPOCH + 1
        mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=fresh_epoch,
            timeout=240,
            step=0.5,
        )

        last_scan_height = btc_rpc.proxy.getblockcount()
        postings = self._checkpoint_postings(
            btc_rpc,
            first_scan_height,
            last_scan_height,
        )
        for epoch in range(1, fresh_epoch + 1):
            txids = postings.get(epoch, [])
            assert len(txids) == 1, (
                f"checkpoint epoch {epoch} appeared {len(txids)} times on L1: {txids}"
            )

        logger.info(
            "epochs 1..%s each appeared exactly once across restart",
            fresh_epoch,
        )
        return True

    @staticmethod
    def _wait_for_epoch_one_seen(strata_rpc):
        info = wait_until_with_value(
            lambda: strata_rpc.strata_getCheckpointInfo(1),
            lambda value: (
                isinstance(value, dict)
                and value.get("confirmation_status", {}).get("status") in {"confirmed", "finalized"}
            ),
            error_with="checkpoint epoch 1 was not observed on L1",
            timeout=120,
            step=0.5,
        )
        logger.info("checkpoint epoch 1 observed with status %s", info["confirmation_status"])

    @staticmethod
    def _wait_for_enqueued_backlog(strata_rpc, admin_rpc, first_epoch, last_epoch):
        def _backlog_state():
            infos = {
                epoch: strata_rpc.strata_getCheckpointInfo(epoch)
                for epoch in range(first_epoch, last_epoch + 1)
            }
            outstanding_epochs = set()
            for duty in admin_rpc.strata_strataadmin_getSequencerDuties():
                if isinstance(duty, dict) and "SignCheckpoint" in duty:
                    outstanding_epochs.add(parse_checkpoint_epoch(duty))
            return infos, outstanding_epochs

        infos, _ = wait_until_with_value(
            _backlog_state,
            lambda value: all(
                isinstance(value[0][epoch], dict)
                and value[0][epoch].get("confirmation_status", {}).get("status") == "pending"
                and epoch not in value[1]
                for epoch in range(first_epoch, last_epoch + 1)
            ),
            error_with=(
                f"checkpoint epochs {first_epoch}..{last_epoch} were not built, signed, and queued"
            ),
            timeout=120,
            step=0.5,
        )
        logger.info(
            "checkpoint backlog is pending without L1 refs: %s",
            sorted(infos),
        )

    @staticmethod
    def _wait_for_checkpoint_in_mempool(btc_rpc, epoch):
        def _find_checkpoint():
            for txid in btc_rpc.proxy.getrawmempool():
                tx = btc_rpc.proxy.getrawtransaction(txid, 1)
                if not TestCheckpointNoDoubleSubmissionOnRestart._is_checkpoint_tx(tx):
                    continue
                payload = extract_posted_checkpoint_payload(btc_rpc, txid)
                if parse_checkpoint_payload(payload).epoch == epoch:
                    return txid
            return None

        txid = wait_until_with_value(
            _find_checkpoint,
            lambda value: value is not None,
            error_with=f"checkpoint epoch {epoch} was not queued in the Bitcoin mempool",
            timeout=120,
            step=0.5,
        )
        logger.info("checkpoint epoch %s is queued in mempool as %s", epoch, txid)

    @staticmethod
    def _checkpoint_postings(btc_rpc, first_height, last_height):
        postings = defaultdict(list)
        for height in range(first_height, last_height + 1):
            block_hash = btc_rpc.proxy.getblockhash(height)
            block = btc_rpc.proxy.getblock(block_hash, 2)
            for tx in block["tx"]:
                if not TestCheckpointNoDoubleSubmissionOnRestart._is_checkpoint_tx(tx):
                    continue
                txid = tx["txid"]
                payload = extract_posted_checkpoint_payload(btc_rpc, txid)
                epoch = parse_checkpoint_payload(payload).epoch
                postings[epoch].append(txid)
        return postings

    @staticmethod
    def _is_checkpoint_tx(tx):
        outputs = tx.get("vout", [])
        if not outputs:
            return False
        script_hex = outputs[0].get("scriptPubKey", {}).get("hex", "")
        try:
            script = bytes.fromhex(script_hex)
        except ValueError:
            return False
        if len(script) < 8 or script[0] != 0x6A:
            return False
        tag = script[2:]
        return (
            len(tag) >= 6
            and tag[4] == CHECKPOINT_SUBPROTOCOL_ID
            and tag[5] == OL_STF_CHECKPOINT_TX_TYPE
        )

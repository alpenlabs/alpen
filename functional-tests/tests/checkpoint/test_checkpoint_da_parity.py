"""OL DA byte parity between checkpoint publication on L1 and the local payload.

Byte target: the inner `sidecar.ol_state_diff` bytes (strata-codec StateDiff /
OLDaPayloadV1 wire format) — not the whole checkpoint payload, whose `proof` may
differ between duty-time and posted forms. The local reference is the
SignCheckpoint duty payload; `new_tip` equality anchors both to the same epoch.

Replay and proof validation are out of scope; see
tests/strata/test_checkpoint_sync_node*.py.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from tests.checkpoint.helpers import (
    extract_posted_checkpoint_payload,
    mine_until_finalized_epoch,
    parse_checkpoint_payload,
    verify_payload_parser_fixture,
    wait_for_checkpoint_duty,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointDaParity(StrataNodeTest):
    """Posted `sidecar.ol_state_diff` bytes match the locally encoded StateDiff bytes."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        verify_payload_parser_fixture()

        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        strata_admin_rpc = strata.create_admin_rpc()
        btc_rpc = bitcoin.create_rpc()

        # Drive L1 forward so OL produces blocks and completes an epoch.
        btc_rpc.proxy.generatetoaddress(5, btc_rpc.proxy.getnewaddress())

        duty = wait_for_checkpoint_duty(strata_admin_rpc, timeout=120, step=0.5, min_epoch=1)
        local_payload = bytes(duty["SignCheckpoint"]["checkpoint"])
        local = parse_checkpoint_payload(local_payload)
        epoch = local.epoch
        logger.info(
            "captured duty payload for epoch %d, ol_state_diff=%d bytes",
            epoch,
            len(local.ol_state_diff),
        )

        # Wait until this epoch's checkpoint is posted and finalized on L1.
        mine_until_finalized_epoch(
            bitcoin=bitcoin, strata=strata, strata_rpc=strata_rpc, target_epoch=epoch
        )

        # Anchor to the exact posted tx via the node's L1 reference rather than a
        # broad tag scan, which could pick up a rejected checkpoint candidate.
        info = strata_rpc.strata_getCheckpointInfo(epoch)
        assert info is not None, f"no checkpoint info for finalized epoch {epoch}"
        status = info["confirmation_status"]
        assert status["status"] == "finalized", f"epoch {epoch} not finalized: {status}"

        posted_payload = extract_posted_checkpoint_payload(btc_rpc, status["l1_reference"]["txid"])
        posted = parse_checkpoint_payload(posted_payload)

        assert posted.new_tip_bytes == local.new_tip_bytes, (
            f"posted new_tip differs from duty new_tip: "
            f"posted={posted.new_tip_bytes.hex()} local={local.new_tip_bytes.hex()}"
        )
        # Every completed epoch consumes new L1 manifests, so the global state
        # diff is non-empty by protocol behavior, not incidental activity.
        assert len(local.ol_state_diff) > 0, "expected non-empty state diff for epoch"
        assert posted.ol_state_diff == local.ol_state_diff, (
            f"posted ol_state_diff ({len(posted.ol_state_diff)} bytes) != "
            f"locally encoded StateDiff ({len(local.ol_state_diff)} bytes)"
        )
        logger.info(
            "epoch %d: posted ol_state_diff matches local encoding (%d bytes)",
            epoch,
            len(local.ol_state_diff),
        )

        return True

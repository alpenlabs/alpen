"""Test sequencer recovers after crash during CSM event processing."""

import logging

import flexitest

from common.config import ServiceType
from common.crash_helpers import CrashTest, crash_and_recover

logger = logging.getLogger(__name__)


@flexitest.register
class TestCrashCsmEvent(CrashTest):
    """Crash at csm_event bail point and verify recovery.

    The CSM worker processes ASM status updates triggered by L1 block arrivals,
    so we mine Bitcoin blocks after arming the bail to ensure the CSM code path
    is actually exercised.
    """

    def main(self, ctx):
        strata = self.get_strata()
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata.wait_for_rpc_ready(timeout=10)

        def mine_l1_blocks_to_trigger_csm() -> None:
            btc_rpc = bitcoin.create_rpc()
            addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(3, addr)

        result = crash_and_recover(
            strata,
            "csm_event",
            after_arm=mine_l1_blocks_to_trigger_csm,
        )

        logger.info(f"Post-recovery height: {result.post_status['tip']['slot']}")
        return True

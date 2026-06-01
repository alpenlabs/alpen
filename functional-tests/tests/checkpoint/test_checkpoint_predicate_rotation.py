"""STR-3130: OL checkpoint predicate rotation is enforced end to end."""

import logging
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import create_checkpoint_predicate_update
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import (
    mine_until_finalized_epoch,
    parse_checkpoint_epoch,
    wait_for_checkpoint_duty,
)

logger = logging.getLogger(__name__)

POST_ADMIN_UPDATE_L1_BLOCKS = 5
PREDICATE_REJECTION_L1_BLOCKS = 8
PREDICATE_SETTLE_TIMEOUT_SECONDS = 120


@flexitest.register
class TestCheckpointPredicateRotation(StrataNodeTest):
    """Rotating the OL checkpoint predicate changes ASM checkpoint acceptance."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                fund_test_cli_wallet=True,
                admin_confirmation_depth=2,
            )
        )

    def main(self, ctx):
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        strata: StrataService = self.get_service(ServiceType.Strata)

        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        mine_addr = btc_rpc.proxy.getnewaddress()

        baseline = mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=1,
            timeout=120,
            step=1.0,
        )
        logger.info("baseline finalized epoch under AlwaysAccept: %s", baseline["epoch"])

        admin_xpriv = self._read_admin_xpriv(strata)
        result = create_checkpoint_predicate_update(
            seq_no=1,
            predicate="NeverAccept",
            admin_xpriv=admin_xpriv,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        logger.info("submitted NeverAccept checkpoint predicate update: %s", result)

        self._mine_l1_and_wait_for_asm(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            blocks=POST_ADMIN_UPDATE_L1_BLOCKS,
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
        )

        activated_finalized_epoch = self._finalized_epoch(strata, strata_rpc)
        blocked_epoch = activated_finalized_epoch + 1
        logger.info(
            "checkpoint predicate update processed; finalized=%s, "
            "expecting epoch %s to remain unfinalized",
            activated_finalized_epoch,
            blocked_epoch,
        )

        duty = wait_for_checkpoint_duty(
            strata_rpc,
            timeout=120,
            step=1.0,
            min_epoch=blocked_epoch,
        )
        duty_epoch = parse_checkpoint_epoch(duty)
        if duty_epoch != blocked_epoch:
            raise AssertionError(
                f"expected next checkpoint duty for epoch {blocked_epoch}, got {duty_epoch}"
            )

        checkpoint_info = self._wait_for_checkpoint_info(strata_rpc, blocked_epoch)
        logger.info(
            "checkpoint epoch %s created with status %s",
            blocked_epoch,
            self._checkpoint_status(checkpoint_info),
        )

        for _ in range(PREDICATE_REJECTION_L1_BLOCKS):
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=1,
                timeout=30,
            )
            finalized_epoch = self._finalized_epoch(strata, strata_rpc)
            if finalized_epoch > activated_finalized_epoch:
                raise AssertionError(
                    "checkpoint finalized after rotating predicate to NeverAccept: "
                    f"before={activated_finalized_epoch}, after={finalized_epoch}"
                )

        checkpoint_info = strata_rpc.strata_getCheckpointInfo(blocked_epoch)
        checkpoint_status = self._checkpoint_status(checkpoint_info)
        if checkpoint_status != "pending":
            raise AssertionError(
                f"expected rejected checkpoint epoch {blocked_epoch} to stay pending, "
                f"got {checkpoint_status!r}"
            )

        logger.info(
            "checkpoint epoch %s stayed pending across %s L1 blocks after predicate rotation",
            blocked_epoch,
            PREDICATE_REJECTION_L1_BLOCKS,
        )
        return True

    @staticmethod
    def _read_admin_xpriv(strata: StrataService) -> str:
        admin_key_path = Path(strata.props["datadir"]) / "bridge-operator_keys"
        if not admin_key_path.exists():
            raise AssertionError(f"admin key file not found: {admin_key_path}")
        admin_xpriv = admin_key_path.read_text().strip()
        if not admin_xpriv:
            raise AssertionError(f"admin key file is empty: {admin_key_path}")
        return admin_xpriv

    @staticmethod
    def _finalized_epoch(strata: StrataService, strata_rpc) -> int:
        return strata.get_sync_status(strata_rpc)["finalized"]["epoch"]

    @staticmethod
    def _mine_l1_and_wait_for_asm(
        bitcoin: BitcoinService,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr,
        blocks: int,
        timeout: int,
    ) -> None:
        start_height = btc_rpc.proxy.getblockcount()
        btc_rpc.proxy.generatetoaddress(blocks, mine_addr)
        strata.wait_for_l1_commitment_at(
            start_height + blocks,
            rpc=strata_rpc,
            timeout=timeout,
            poll_interval=0.5,
        )

    @staticmethod
    def _wait_for_checkpoint_info(strata_rpc, epoch: int) -> dict:
        return wait_until_with_value(
            lambda: strata_rpc.strata_getCheckpointInfo(epoch),
            lambda info: info is not None,
            error_with=f"checkpoint info for epoch {epoch} was not created",
            timeout=120,
            step=1.0,
        )

    @staticmethod
    def _checkpoint_status(checkpoint_info: dict | None) -> str | None:
        if checkpoint_info is None:
            return None

        status = checkpoint_info.get("confirmation_status")
        if isinstance(status, str):
            return status.lower()
        if isinstance(status, dict):
            return status.get("status")
        return None

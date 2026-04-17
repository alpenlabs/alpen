"""EE fullnode sync after an EE predicate rotation."""

import contextlib
import logging
import tempfile
from pathlib import Path

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import create_ee_predicate_update
from common.wait import wait_until_with_value
from envconfigs.el_ol import EeOLEnv
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair

logger = logging.getLogger(__name__)

INITIAL_EE_BLOCKS = 5
POST_ADMIN_UPDATE_L1_BLOCKS = 5
PREDICATE_SETTLE_TIMEOUT_SECONDS = 120
HISTORICAL_EE_BLOCKS_AFTER_ROTATION = 8
FRESH_EE_BLOCKS_AFTER_LATE_JOIN = 3

# Initial Alpen account predicate matches `EeAcctProgram::test_predicate_key()`
# (deterministic test SK = [0x02; 32] in strata_proofimpl_alpen_acct).
INITIAL_ACCT_PREDICATE = (
    "Bip340Schnorr:4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766"
)


@flexitest.register
class TestEePredicateFullnodeSync(BaseTest):
    """Tests late EE fullnode sync after the Alpen update predicate rotates."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLEnv(
                pre_generate_blocks=110,
                admin_confirmation_depth=2,
                fund_test_cli_wallet=True,
            )
        )

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        ee_fullnode_0: AlpenClientService = self.get_service(ServiceType.AlpenFullNode)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=30)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=30,
        )

        alpen_seq.wait_for_peers(1, timeout=60)
        ee_fullnode_0.wait_for_peers(1, timeout=60)
        alpen_seq.wait_for_block(INITIAL_EE_BLOCKS, timeout=120)
        ee_fullnode_0.wait_for_block(INITIAL_EE_BLOCKS, timeout=120)

        admin_xpriv = self._read_admin_xpriv(strata_seq)
        btc_url = bitcoin.props["rpc_url"]
        btc_user = bitcoin.props["rpc_user"]
        btc_password = bitcoin.props["rpc_password"]
        mine_addr = btc_rpc.proxy.getnewaddress()

        initial_vk = strata_rpc.strata_getSnarkAccountStateByTag(ALPEN_ACCOUNT_ID, "latest")[
            "update_vk"
        ]
        if initial_vk != INITIAL_ACCT_PREDICATE:
            raise AssertionError(
                f"expected initial update_vk to be {INITIAL_ACCT_PREDICATE!r}, got {initial_vk!r}"
            )

        self._apply_predicate_update(
            seq_no=1,
            target="NeverAccept",
            admin_xpriv=admin_xpriv,
            btc_url=btc_url,
            btc_user=btc_user,
            btc_password=btc_password,
            btc_rpc=btc_rpc,
            strata_rpc=strata_rpc,
            mine_addr=mine_addr,
        )

        historical_target = alpen_seq.get_block_number() + HISTORICAL_EE_BLOCKS_AFTER_ROTATION
        alpen_seq.wait_for_block(historical_target, timeout=120)
        ee_fullnode_0.wait_for_block(historical_target, timeout=120)
        historical_hash = ee_fullnode_0.get_block_by_number(historical_target)["hash"]
        late_join_start_height = alpen_seq.get_block_number()
        if historical_target > late_join_start_height:
            raise AssertionError(
                f"historical replay target {historical_target} is after late-join height "
                f"{late_join_start_height}"
            )
        late_join_start_hash = ee_fullnode_0.get_block_by_number(late_join_start_height)["hash"]

        _, sequencer_pubkey = generate_sequencer_keypair()
        factory = AlpenClientFactory(range(31003, 31103))
        fn0_enode = ee_fullnode_0.get_enode()

        tmpdir = tempfile.mkdtemp(prefix="alpen_fullnode_after_vk_rotation_")
        if any(Path(tmpdir).iterdir()):
            raise AssertionError(f"late fullnode datadir is not empty before startup: {tmpdir}")

        ee_fullnode_1 = None
        try:
            ee_fullnode_1 = factory.create_fullnode(
                sequencer_pubkey=sequencer_pubkey,
                trusted_peers=[fn0_enode],
                bootnodes=None,
                enable_discovery=False,
                instance_id=1,
                datadir_override=tmpdir,
                sequencer_http=alpen_seq.props["http_url"],
                ol_endpoint=strata_seq.props["rpc_url"],
            )
            ee_fullnode_1.wait_for_ready(timeout=30)

            fn0_rpc = ee_fullnode_0.create_rpc()
            fn0_rpc.admin_addPeer(ee_fullnode_1.get_enode())

            ee_fullnode_1.wait_for_peers(1, timeout=30)
            ee_fullnode_1.wait_for_block_hash(historical_target, historical_hash, timeout=120)
            ee_fullnode_1.wait_for_block_hash(
                late_join_start_height,
                late_join_start_hash,
                timeout=120,
            )

            fresh_target = historical_target + FRESH_EE_BLOCKS_AFTER_LATE_JOIN
            alpen_seq.wait_for_block(fresh_target, timeout=120)
            ee_fullnode_0.wait_for_block(fresh_target, timeout=120)
            fresh_hash = ee_fullnode_0.get_block_by_number(fresh_target)["hash"]
            ee_fullnode_1.wait_for_block_hash(fresh_target, fresh_hash, timeout=120)

            logger.info(
                "fresh EE fullnode replayed through late-join block %s "
                "and synced fresh block %s after VK rotation",
                historical_target,
                fresh_target,
            )
            return True
        finally:
            if ee_fullnode_1 is not None:
                with contextlib.suppress(Exception):
                    ee_fullnode_1.stop()

    @staticmethod
    def _read_admin_xpriv(strata_seq: StrataService) -> str:
        admin_key_path = Path(strata_seq.props["datadir"]) / "bridge-operator_keys"
        if not admin_key_path.exists():
            raise AssertionError(f"admin key file not found: {admin_key_path}")
        admin_xpriv = admin_key_path.read_text().strip()
        if not admin_xpriv:
            raise AssertionError(f"admin key file is empty: {admin_key_path}")
        return admin_xpriv

    @staticmethod
    def _apply_predicate_update(
        seq_no: int,
        target: str,
        admin_xpriv: str,
        btc_url: str,
        btc_user: str,
        btc_password: str,
        btc_rpc,
        strata_rpc,
        mine_addr,
    ) -> None:
        result = create_ee_predicate_update(
            seq_no=seq_no,
            predicate=target,
            admin_xpriv=admin_xpriv,
            btc_url=btc_url,
            btc_user=btc_user,
            btc_password=btc_password,
        )
        logger.info("applied %s update (seq %d): %s", target, seq_no, result)

        btc_rpc.proxy.generatetoaddress(POST_ADMIN_UPDATE_L1_BLOCKS, mine_addr)

        def fetch_update_vk_and_mine() -> str:
            btc_rpc.proxy.generatetoaddress(1, mine_addr)
            return strata_rpc.strata_getSnarkAccountStateByTag(ALPEN_ACCOUNT_ID, "latest")[
                "update_vk"
            ]

        wait_until_with_value(
            fetch_update_vk_and_mine,
            lambda vk: vk == target,
            error_with=f"update_vk did not transition to {target} in OL state",
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
        )
        logger.info("update_vk transitioned to %s", target)

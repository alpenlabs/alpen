"""Look up an update through Bitcoin RPC, sign its cancellation, and broadcast it."""

import json
import os
import subprocess
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.test_cli import _run_command, create_checkpoint_predicate_update
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch


@flexitest.register
class TestAdminCancel(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                fund_test_cli_wallet=True,
                admin_confirmation_depth=24,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)
        btc = bitcoin.create_rpc().proxy
        rpc = strata.wait_for_rpc_ready(timeout=20)
        address = btc.getnewaddress()
        datadir = Path(strata.props["datadir"])
        key = (datadir / "bridge-operator_keys").read_text().strip()
        baseline = mine_until_finalized_epoch(bitcoin, strata, rpc, 1)
        create_checkpoint_predicate_update(
            seq_no=1,
            predicate="AlwaysAccept",
            admin_xpriv=key,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        btc.generatetoaddress(1, address)
        created = create_checkpoint_predicate_update(
            seq_no=2,
            predicate="NeverAccept",
            admin_xpriv=key,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )

        def lookup(txid):
            return json.loads(
                _run_command(
                    [
                        "lookup-admin-update",
                        "--txid",
                        txid,
                        "--asm-params",
                        str(datadir / "asm-params.json"),
                        "--btc-url",
                        bitcoin.props["rpc_url"],
                        "--btc-user",
                        bitcoin.props["rpc_user"],
                        "--btc-password",
                        bitcoin.props["rpc_password"],
                    ]
                )
            )

        def mine_and_lookup():
            btc.generatetoaddress(1, address)
            return lookup(created["reveal_txid"])

        snapshot = wait_until_with_value(
            mine_and_lookup,
            lambda update: update["status"] == "pending",
            timeout=60,
            step=0.5,
            error_with="update was not queued",
        )
        update = snapshot
        assert snapshot["l1_block_id"] == btc.getblockhash(snapshot["l1_height"])
        assert update["update_type"] == "OL STF VK Update"
        assert update["authorized_by"] == "Strata Administrator"
        assert update["predicate"] == "NeverAccept"
        assert update["activation_height"] > snapshot["l1_height"]
        assert update["update_id"] == 1
        assert update["last_seq_no"] == 2
        rejected = create_checkpoint_predicate_update(
            seq_no=2,
            predicate="NeverAccept",
            admin_xpriv=key,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        btc.generatetoaddress(1, address)
        try:
            lookup(rejected["reveal_txid"])
        except RuntimeError as error:
            assert "target transaction was rejected" in str(error)
        else:
            raise AssertionError("lookup assigned an ID to a rejected update")
        assert lookup(created["reveal_txid"])["last_seq_no"] == 2
        target = str(update["update_id"])
        signed = subprocess.run(
            [
                "strata-test-cli",
                "sign-admin-action",
                "--update",
                "ol-stf-vk",
                "--network",
                "regtest",
                "--seq-no",
                "3",
                "--predicate",
                update["predicate"],
                "--cancel-target-id",
                target,
            ],
            env={**os.environ, "STRATA_ADMIN_KEY": key},
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        assert "Signed with public key:" in signed.stderr
        assert "Action: Cancel" in signed.stderr
        assert f"Target Id: {target}" in signed.stderr
        assert "Target Update: OL STF VK Update" in signed.stderr
        _run_command(
            [
                "create-admin-cancel",
                "--target-id",
                target,
                "--update",
                "ol-stf-vk",
                "--predicate",
                update["predicate"],
                "--seq-no",
                "3",
                "--signers-config",
                str(datadir / "asm-params.json"),
                "--signature",
                f"0:{signed.stdout.strip()}",
                "--btc-url",
                bitcoin.props["rpc_url"],
                "--btc-user",
                bitcoin.props["rpc_user"],
                "--btc-password",
                bitcoin.props["rpc_password"],
            ]
        )
        cancelled = wait_until_with_value(
            mine_and_lookup,
            lambda update: update["status"] == "not_pending",
            timeout=60,
            step=0.5,
            error_with="cancellation did not remove the update",
        )
        assert cancelled["l1_height"] < update["activation_height"]
        while btc.getblockcount() <= update["activation_height"]:
            mine_and_lookup()

        sealed = rpc.strata_getChainStatus()["latest"]["epoch"]
        finalized = mine_until_finalized_epoch(bitcoin, strata, rpc, sealed + 2)
        assert finalized["epoch"] > baseline["epoch"]
        return True

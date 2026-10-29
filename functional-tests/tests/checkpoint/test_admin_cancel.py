"""Sign and broadcast a cancellation with an explicitly supplied UpdateId."""

import json
import os
import subprocess
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.test_cli import _run_command, create_checkpoint_predicate_update
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
        datadir = Path(strata.props["datadir"])
        key = (datadir / "bridge-operator_keys").read_text().strip()
        mine_until_finalized_epoch(bitcoin, strata, rpc, 1)

        def confirm(txid):
            blockhash = bitcoin.mine_until(
                check=lambda: btc.getrawtransaction(txid, 1).get("blockhash"),
                predicate=lambda blockhash: blockhash is not None,
                timeout=30,
                step=0.5,
            )
            height = btc.getblock(blockhash)["height"]
            strata.wait_for_asm_manifest_commitment_at(height, rpc=rpc, timeout=30)
            return height

        update = create_checkpoint_predicate_update(
            seq_no=1,
            predicate="NeverAccept",
            admin_xpriv=key,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        activation_height = confirm(update["reveal_txid"]) + 24
        # This is the first admin update in the fresh test chain, so its ID is 0.
        signed = subprocess.run(
            [
                "strata-test-cli",
                "sign-admin-action",
                "--update",
                "ol-stf-vk",
                "--network",
                "regtest",
                "--seq-no",
                "2",
                "--predicate",
                "NeverAccept",
                "--cancel-target-id",
                "0",
            ],
            env={**os.environ, "STRATA_ADMIN_KEY": key},
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        assert "Signed with public key:" in signed.stderr
        assert "Action: Cancel" in signed.stderr
        assert "Target Id: 0" in signed.stderr
        assert "Target Update: OL STF VK Update" in signed.stderr
        cancelled = json.loads(
            _run_command(
                [
                    "create-admin-cancel",
                    "--target-id",
                    "0",
                    "--update",
                    "ol-stf-vk",
                    "--predicate",
                    "NeverAccept",
                    "--seq-no",
                    "2",
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
        )
        assert confirm(cancelled["reveal_txid"]) < activation_height
        btc.generatetoaddress(activation_height + 1 - btc.getblockcount(), btc.getnewaddress())
        strata.wait_for_asm_manifest_commitment_at(activation_height + 1, rpc=rpc, timeout=30)
        sealed = rpc.strata_getChainStatus()["latest"]["epoch"]
        finalized = mine_until_finalized_epoch(bitcoin, strata, rpc, sealed + 2)
        checkpoint = rpc.strata_getCheckpointInfo(finalized["epoch"])
        assert checkpoint["l1_range"][1]["height"] > activation_height
        return True

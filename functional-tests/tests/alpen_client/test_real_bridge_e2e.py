"""Real bridge deposit into an unfunded Alpen EE account."""

import json
import logging
import os
import re
import subprocess
import time
from pathlib import Path

import flexitest

from common.accounts import ManagedAccount, get_recipient_account
from common.config.constants import DEV_ADDRESS, DEV_PRIVATE_KEY, SATS_TO_WEI, ServiceType
from common.evm_utils import get_balance, send_raw_transaction, wait_for_receipt
from common.external_bridge import (
    compute_bridge_aggregate_pubkey,
    find_bridge_repo,
    load_bridge_operator_pubkeys,
    start_external_bridge,
)
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from envconfigs.real_bridge import RealBridgeEeOLEnv

logger = logging.getLogger(__name__)

OPERATOR_COUNT = int(os.environ.get("ALPEN_EXTERNAL_BRIDGE_OPERATORS", "2"))
CLI_SEED_HEX = "11" * 16
SPEND_VALUE_WEI = 100_000_000_000_000_000


@flexitest.register
class RealBridgeDepositToEeTest(flexitest.Test):
    """Deposit through strata-bridge, then spend the bridged EE funds."""

    def __init__(self, ctx: flexitest.InitContext):
        self.bridge_repo = find_bridge_repo()
        self.operator_pubkeys = load_bridge_operator_pubkeys(self.bridge_repo, OPERATOR_COUNT)
        ctx.set_env(
            RealBridgeEeOLEnv(
                bridge_operator_pubkeys=self.operator_pubkeys,
                seal_epoch_slots=4,
            )
        )

    def premain(self, ctx: flexitest.RunContext):
        self.runctx = ctx

    def main(self, ctx):
        alpen_seq: AlpenClientService = ctx.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = ctx.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = ctx.get_service(ServiceType.Bitcoin)

        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=30)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            "01" * 32,
            rpc=strata_rpc,
            timeout=30,
        )
        alpen_seq.wait_for_block(3, timeout=60)

        evm_rpc = alpen_seq.create_rpc()
        initial_balance = get_balance(evm_rpc, DEV_ADDRESS)
        assert initial_balance == 0, f"expected no prefunded EE balance, got {initial_balance}"

        env_dir = Path(strata_seq.props["datadir"]).parent
        rollup_params_path = Path(strata_seq.props["datadir"]) / "rollup-params.json"
        rollup_params = json.loads(rollup_params_path.read_text())
        deposit_sats = int(rollup_params["deposit_amount"])
        expected_balance = deposit_sats * SATS_TO_WEI
        genesis_l1_height = int(rollup_params["genesis_l1_view"]["blk"]["height"])

        bridge = start_external_bridge(
            self.bridge_repo,
            env_dir,
            dict(bitcoin.props),
            genesis_l1_height,
            operator_count=OPERATOR_COUNT,
            deposit_amount=deposit_sats,
        )

        try:
            drt_txid = self._send_deposit_with_alpen_cli(
                bitcoin,
                alpen_seq,
                env_dir,
                rollup_params_path,
            )
            logger.info("Broadcasted alpen-cli DRT %s", drt_txid)

            btc_rpc = bitcoin.create_rpc()
            mine_addr = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(2, mine_addr)

            deposit_info = bridge.wait_deposit_complete(drt_txid, timeout=420)
            logger.info("Bridge deposit completed: %s", deposit_info)

            balance = self._wait_for_ee_deposit(
                bitcoin,
                alpen_seq,
                DEV_ADDRESS,
                expected_balance,
            )
            logger.info("EE balance after bridge deposit: %s", balance)

            self._spend_ee_funds(evm_rpc)
            confirmed_epoch = self._wait_for_confirmed_epoch(strata_seq, strata_rpc, bitcoin, 6)
            logger.info("Confirmed epoch after bridge deposit: %s", confirmed_epoch)
        finally:
            bridge.stop()

        return True

    def _run_alpen(
        self,
        args: list[str],
        env_dir: Path,
        env: dict[str, str],
        include_stderr: bool = False,
    ) -> str:
        result = subprocess.run(
            ["alpen", *args],
            cwd=env_dir,
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"alpen {' '.join(args)} failed with {result.returncode}\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        if include_stderr:
            return result.stdout + "\n" + result.stderr
        return result.stdout

    def _send_deposit_with_alpen_cli(
        self,
        bitcoin: BitcoinService,
        alpen_seq: AlpenClientService,
        env_dir: Path,
        rollup_params_path: Path,
    ) -> str:
        cli_dir = env_dir / "alpen_cli"
        config_dir = cli_dir / "config"
        data_dir = cli_dir / "data"
        config_dir.mkdir(parents=True, exist_ok=True)
        data_dir.mkdir(parents=True, exist_ok=True)

        bridge_pubkey = compute_bridge_aggregate_pubkey(self.operator_pubkeys)
        config_path = config_dir / "config.toml"
        config_path.write_text(
            "\n".join(
                [
                    f'bitcoind_rpc_user = "{bitcoin.props["rpc_user"]}"',
                    f'bitcoind_rpc_pw = "{bitcoin.props["rpc_password"]}"',
                    f'bitcoind_rpc_endpoint = "http://127.0.0.1:{bitcoin.props["rpc_port"]}"',
                    f'alpen_endpoint = "{alpen_seq.props["http_url"]}"',
                    'faucet_endpoint = ""',
                    f'bridge_pubkey = "{bridge_pubkey}"',
                    "bridge_fee_sats = 1000",
                    "finality_depth = 1",
                    f'rollup_params_path = "{rollup_params_path}"',
                    f'seed = "{CLI_SEED_HEX}"',
                    "",
                ]
            )
        )

        env = os.environ.copy()
        env["CLI_CONFIG"] = str(config_path)
        env["PROJ_DIRS"] = str(cli_dir)
        env["STRATA_NETWORK_PARAMS"] = str(rollup_params_path)

        receive_out = self._run_alpen(["receive", "signet"], env_dir, env)
        receive_address = receive_out.strip().splitlines()[-1]
        btc_rpc = bitcoin.create_rpc()
        btc_rpc.proxy.sendtoaddress(receive_address, 11)
        btc_rpc.proxy.generatetoaddress(8, btc_rpc.proxy.getnewaddress())

        deposit_out = self._run_alpen(
            ["deposit", DEV_ADDRESS, "--fee-rate", "1"],
            env_dir,
            env,
            include_stderr=True,
        )
        transaction_id_match = re.search(r"Transaction ID:\s*([0-9a-f]{64})", deposit_out)
        if transaction_id_match is not None:
            return transaction_id_match.group(1)

        recovery_pk_match = re.search(r"Recovery public key:\s*([0-9a-f]{64})", deposit_out)
        if recovery_pk_match is not None:
            return self._find_drt_by_recovery_pk(bitcoin, recovery_pk_match.group(1))

        raise RuntimeError(f"failed to parse DRT txid from alpen output:\n{deposit_out}")

    def _find_drt_by_recovery_pk(self, bitcoin: BitcoinService, recovery_pk: str) -> str:
        btc_rpc = bitcoin.create_rpc()
        mempool_txids = btc_rpc.proxy.getrawmempool()
        for txid in mempool_txids:
            tx = btc_rpc.proxy.getrawtransaction(txid, True)
            for vout in tx.get("vout", []):
                script_pubkey = vout.get("scriptPubKey", {})
                if recovery_pk in script_pubkey.get("asm", ""):
                    return str(txid)

        raise RuntimeError(f"failed to find DRT with recovery public key {recovery_pk}")

    def _wait_for_ee_deposit(
        self,
        bitcoin: BitcoinService,
        alpen_seq: AlpenClientService,
        address: str,
        expected_balance: int,
    ) -> int:
        btc_rpc = bitcoin.create_rpc()
        evm_rpc = alpen_seq.create_rpc()
        mine_addr = btc_rpc.proxy.getnewaddress()

        def mine_and_check_balance():
            btc_rpc.proxy.generatetoaddress(2, mine_addr)
            time.sleep(2)
            return get_balance(evm_rpc, address)

        return wait_until_with_value(
            mine_and_check_balance,
            lambda balance: balance >= expected_balance,
            error_with=f"EE balance did not reach bridged amount {expected_balance}",
            timeout=420,
            step=1,
        )

    def _spend_ee_funds(self, evm_rpc) -> None:
        sender = ManagedAccount.from_key(DEV_PRIVATE_KEY)
        nonce = int(evm_rpc.eth_getTransactionCount(sender.address, "pending"), 16)
        sender.sync_nonce(nonce)
        recipient = get_recipient_account()
        gas_price = int(evm_rpc.eth_gasPrice(), 16)
        raw_tx = sender.sign_transfer(
            to=recipient.address,
            value=SPEND_VALUE_WEI,
            gas_price=gas_price,
        )
        tx_hash = send_raw_transaction(evm_rpc, raw_tx)
        receipt = wait_for_receipt(evm_rpc, tx_hash, timeout=60)
        assert int(receipt["status"], 16) == 1, f"spend failed: {receipt}"
        recipient_balance = get_balance(evm_rpc, recipient.address)
        assert recipient_balance >= SPEND_VALUE_WEI

    def _wait_for_confirmed_epoch(
        self,
        strata_seq: StrataService,
        strata_rpc,
        bitcoin: BitcoinService,
        target_epoch: int,
    ) -> int:
        btc_rpc = bitcoin.create_rpc()
        mine_addr = btc_rpc.proxy.getnewaddress()

        def mine_and_check_epoch():
            btc_rpc.proxy.generatetoaddress(2, mine_addr)
            time.sleep(2)
            status = strata_seq.get_sync_status(strata_rpc)
            return status["confirmed"]["epoch"]

        return wait_until_with_value(
            mine_and_check_epoch,
            lambda epoch: epoch >= target_epoch,
            error_with=f"confirmed epoch did not reach {target_epoch}",
            timeout=420,
            step=1,
        )

"""Verify Safe Singleton Factory deployment by transaction after EE startup."""

import json
import logging
import tempfile
from pathlib import Path

import flexitest

from common.accounts import ManagedAccount
from common.base_test import BaseTest
from common.config.constants import DEV_PRIVATE_KEY, ServiceType
from common.evm_utils import send_raw_transaction, wait_for_receipt
from common.safe_factory import (
    SAFE_FACTORY_ADDRESS,
    SAFE_FACTORY_DEPLOYER_ADDRESS,
    SAFE_FACTORY_DEPLOYMENT_CHAIN_ID,
    SAFE_FACTORY_DEPLOYMENT_TX,
    SAFE_FACTORY_RUNTIME,
)
from common.safe_factory_create2 import (
    CREATE2_SALT,
    TINY_RUNTIME_HEX,
    create2_address,
    tiny_init_code,
)
from common.services.alpen_client import AlpenClientService
from envconfigs.el_ol import EeOLEnv

logger = logging.getLogger(__name__)

REPO_ROOT = Path(__file__).resolve().parents[3]
DEV_CHAIN_SPEC_PATH = REPO_ROOT / "crates/reth/chainspec/src/res/alpen-dev-chain.json"
FUNDED_BALANCE = "0xD3C21BCECCEDA1000000"


def _write_genesis_for_safe_tx_deploy() -> str:
    with DEV_CHAIN_SPEC_PATH.open() as genesis_file:
        genesis = json.load(genesis_file)

    genesis["config"]["chainId"] = SAFE_FACTORY_DEPLOYMENT_CHAIN_ID
    alloc = genesis.setdefault("alloc", {})
    alloc[SAFE_FACTORY_DEPLOYER_ADDRESS] = {"balance": FUNDED_BALANCE}

    with tempfile.NamedTemporaryFile(
        "w",
        prefix="safe_factory_tx_genesis_",
        suffix=".json",
        delete=False,
    ) as merged_genesis_file:
        json.dump(genesis, merged_genesis_file, indent=4)
        merged_genesis_file.write("\n")
        return merged_genesis_file.name


@flexitest.register
class TestSafeFactoryTx(BaseTest):
    """Checks the live-network transaction deployment path in a local EE env."""

    def __init__(self, ctx: flexitest.InitContext):
        self.genesis_path = Path(_write_genesis_for_safe_tx_deploy())
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                custom_chain=str(self.genesis_path),
            )
        )

    def main(self, ctx):  # noqa: ARG002
        try:
            alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
            alpen_seq.wait_for_block(1, timeout=60)

            rpc = alpen_seq.create_rpc()
            chain_id = int(rpc.eth_chainId(), 16)
            assert chain_id == SAFE_FACTORY_DEPLOYMENT_CHAIN_ID, (
                f"chain ID mismatch: got {chain_id}, expected {SAFE_FACTORY_DEPLOYMENT_CHAIN_ID}"
            )

            factory_code_before = rpc.eth_getCode(SAFE_FACTORY_ADDRESS, "latest")
            assert factory_code_before == "0x", (
                f"Safe factory unexpectedly exists before tx deploy: {factory_code_before}"
            )

            tx_hash = send_raw_transaction(rpc, SAFE_FACTORY_DEPLOYMENT_TX)
            receipt = wait_for_receipt(rpc, tx_hash, timeout=120)
            assert receipt["status"] == "0x1", f"Safe factory deployment tx failed: {receipt}"

            contract_address = receipt.get("contractAddress")
            assert contract_address and contract_address.lower() == SAFE_FACTORY_ADDRESS.lower(), (
                f"deployment address mismatch: got {contract_address}, "
                f"expected {SAFE_FACTORY_ADDRESS}"
            )

            factory_code_after = rpc.eth_getCode(SAFE_FACTORY_ADDRESS, "latest")
            assert factory_code_after.lower() == SAFE_FACTORY_RUNTIME.lower(), (
                f"Safe factory code mismatch: got {factory_code_after}, "
                f"expected {SAFE_FACTORY_RUNTIME}"
            )

            init_code = tiny_init_code()
            deployed_address = create2_address(SAFE_FACTORY_ADDRESS, CREATE2_SALT, init_code)
            predeploy_code = rpc.eth_getCode(deployed_address, "latest")
            assert predeploy_code == "0x", (
                f"CREATE2 target {deployed_address} already has code: {predeploy_code}"
            )

            dev_account = ManagedAccount.from_key(
                DEV_PRIVATE_KEY,
                chain_id=SAFE_FACTORY_DEPLOYMENT_CHAIN_ID,
            )
            dev_account.sync_nonce(
                int(rpc.eth_getTransactionCount(dev_account.address, "pending"), 16)
            )
            gas_price = int(rpc.eth_gasPrice(), 16)
            calldata = CREATE2_SALT + init_code
            raw_create2_tx = dev_account.sign_transaction(
                to=SAFE_FACTORY_ADDRESS,
                data=calldata,
                gas_price=gas_price,
                gas=200_000,
            )
            create2_tx_hash = send_raw_transaction(rpc, raw_create2_tx)
            create2_receipt = wait_for_receipt(rpc, create2_tx_hash, timeout=120)
            assert create2_receipt["status"] == "0x1", (
                f"Safe factory CREATE2 tx failed: {create2_receipt}"
            )

            deployed_code = rpc.eth_getCode(deployed_address, "latest")
            assert deployed_code.lower() == TINY_RUNTIME_HEX, (
                f"CREATE2 deployed code mismatch at {deployed_address}: "
                f"got {deployed_code}, expected {TINY_RUNTIME_HEX}"
            )
            logger.info(
                "Safe factory tx deploy tx=%s address=%s code=%s; CREATE2 tx=%s address=%s code=%s",
                tx_hash,
                contract_address,
                factory_code_after,
                create2_tx_hash,
                deployed_address,
                deployed_code,
            )
            self.result_msg = (
                f"Safe factory tx deploy tx={tx_hash} status={receipt['status']} "
                f"address={contract_address}; "
                f"eth_getCode({SAFE_FACTORY_ADDRESS}, latest) -> {factory_code_after}; "
                f"CREATE2 tx={create2_tx_hash} status={create2_receipt['status']} "
                f"address={deployed_address} code={deployed_code}"
            )

            return True
        finally:
            self.genesis_path.unlink(missing_ok=True)

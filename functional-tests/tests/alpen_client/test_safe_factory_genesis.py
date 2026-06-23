"""Verify the Safe Singleton Factory can be predeployed through EE genesis."""

import json
import logging
import tempfile
from pathlib import Path

import flexitest

from common.accounts import get_dev_account
from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.evm_utils import send_raw_transaction, wait_for_receipt
from common.safe_factory import SAFE_FACTORY_ADDRESS, SAFE_FACTORY_RUNTIME
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


def _write_genesis_with_safe_factory() -> str:
    with DEV_CHAIN_SPEC_PATH.open() as genesis_file:
        genesis = json.load(genesis_file)

    alloc = genesis.setdefault("alloc", {})
    alloc[SAFE_FACTORY_ADDRESS] = {
        "balance": "0x0",
        "nonce": "0x1",
        "code": SAFE_FACTORY_RUNTIME,
    }

    with tempfile.NamedTemporaryFile(
        "w",
        prefix="safe_factory_genesis_",
        suffix=".json",
        delete=False,
    ) as merged_genesis_file:
        json.dump(genesis, merged_genesis_file, indent=4)
        merged_genesis_file.write("\n")
        return merged_genesis_file.name


@flexitest.register
class TestSafeFactoryGenesis(BaseTest):
    """Checks the Safe Singleton Factory genesis alloc and CREATE2 behavior."""

    def __init__(self, ctx: flexitest.InitContext):
        self.genesis_path = Path(_write_genesis_with_safe_factory())
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

            factory_code = rpc.eth_getCode(SAFE_FACTORY_ADDRESS, "latest")
            assert factory_code.lower() == SAFE_FACTORY_RUNTIME.lower(), (
                f"Safe factory code mismatch: got {factory_code}, expected {SAFE_FACTORY_RUNTIME}"
            )
            logger.info(
                "eth_getCode(%s) -> %s (matches expected)",
                SAFE_FACTORY_ADDRESS,
                factory_code,
            )

            init_code = tiny_init_code()
            deployed_address = create2_address(SAFE_FACTORY_ADDRESS, CREATE2_SALT, init_code)
            predeploy_code = rpc.eth_getCode(deployed_address, "latest")
            assert predeploy_code == "0x", (
                f"CREATE2 target {deployed_address} already has code: {predeploy_code}"
            )

            dev_account = get_dev_account(rpc)
            gas_price = int(rpc.eth_gasPrice(), 16)
            calldata = CREATE2_SALT + init_code
            raw_tx = dev_account.sign_transaction(
                to=SAFE_FACTORY_ADDRESS,
                data=calldata,
                gas_price=gas_price,
                gas=200_000,
            )

            tx_hash = send_raw_transaction(rpc, raw_tx)
            receipt = wait_for_receipt(rpc, tx_hash, timeout=120)
            assert receipt["status"] == "0x1", f"Safe factory CREATE2 tx failed: {receipt}"

            deployed_code = rpc.eth_getCode(deployed_address, "latest")
            assert deployed_code.lower() == TINY_RUNTIME_HEX, (
                f"CREATE2 deployed code mismatch at {deployed_address}: "
                f"got {deployed_code}, expected {TINY_RUNTIME_HEX}"
            )
            logger.info(
                "Safe factory CREATE2 tx=%s status=%s address=%s code=%s",
                tx_hash,
                receipt["status"],
                deployed_address,
                deployed_code,
            )
            self.result_msg = (
                f"eth_getCode({SAFE_FACTORY_ADDRESS}, latest) -> {factory_code}; "
                f"CREATE2 tx={tx_hash} status={receipt['status']} "
                f"address={deployed_address} code={deployed_code}"
            )

            return True
        finally:
            self.genesis_path.unlink(missing_ok=True)

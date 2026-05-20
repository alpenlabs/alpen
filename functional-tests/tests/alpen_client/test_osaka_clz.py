"""Verify OP_CLZ executes consistently through EE execution and OL proving."""

import logging
import re
from pathlib import Path

import flexitest
from eth_account import Account
from eth_utils import to_checksum_address

from common.accounts import get_dev_account
from common.base_test import BaseTest
from common.config.constants import DEV_CHAIN_ID, DEV_PRIVATE_KEY, ServiceType
from common.evm_utils import send_raw_transaction, wait_for_receipt
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.wait import wait_until_with_value
from envconfigs.el_ol import EeOLEnv

logger = logging.getLogger(__name__)

CLZ_EXPECTED_RESULT = 255
BATCH_SEALING_BLOCK_COUNT = 3
SIGNAL_TIMEOUT_SECS = 180
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")
_SNARK_UPDATE_RE = re.compile(r"submitted snark update to OL\b.*seq_no=(\d+)")


def _clz_runtime() -> bytes:
    # Runtime program:
    #   CLZ(1) == 255 for a 256-bit EVM word.
    #   Store the result in slot 0 so the test can verify it over eth_getStorageAt.
    #
    # Without Osaka active, 0x1E is INVALID, this transaction reverts, and the
    # storage assertion below fails before the prover path is considered.
    return bytes([0x7F]) + (1).to_bytes(32, "big") + bytes([0x1E, 0x60, 0x00, 0x55, 0x00])


def _creation_code(runtime: bytes) -> bytes:
    # Minimal init code that copies `runtime` from the tail of the creation
    # bytecode into memory and returns it as the deployed contract code.
    prefix_size = 12
    if len(runtime) > 0xFF:
        raise ValueError("test runtime must fit in PUSH1 length")
    init_code = bytearray()
    init_code += bytes([0x60, len(runtime)])
    init_code += bytes([0x60, prefix_size])
    init_code += bytes([0x60, 0x00])
    init_code += bytes([0x39])
    init_code += bytes([0x60, len(runtime)])
    init_code += bytes([0x60, 0x00])
    init_code += bytes([0xF3])
    assert len(init_code) == prefix_size
    init_code += runtime
    return bytes(init_code)


def _sign_deploy(nonce: int, data: bytes, gas_price: int) -> str:
    tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": 120_000,
        "to": None,
        "value": 0,
        "data": data,
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
    return "0x" + signed.raw_transaction.hex()


def _ee_log_path(alpen_service: AlpenClientService) -> Path:
    return Path(alpen_service.props["datadir"]) / "service.log"


def _read_log_fragment(log_path: Path, after_offset: int = 0) -> str:
    if not log_path.exists():
        return ""
    with log_path.open("rb") as fh:
        fh.seek(after_offset)
        body = fh.read().decode(errors="replace")
    return _ANSI_RE.sub("", body)


def _submitted_update_seq_nos(log_path: Path, after_offset: int = 0) -> list[int]:
    fragment = _read_log_fragment(log_path, after_offset)
    return [int(match.group(1)) for match in _SNARK_UPDATE_RE.finditer(fragment)]


def _update_seq_no_for_ee_block(block_number: int) -> int:
    if block_number <= 0:
        raise ValueError(f"cannot map genesis block {block_number} to a snark update")
    return (block_number - 1) // BATCH_SEALING_BLOCK_COUNT


def _batch_end_block_number(block_number: int) -> int:
    if block_number <= 0:
        raise ValueError(f"cannot map genesis block {block_number} to a batch")
    return ((block_number - 1) // BATCH_SEALING_BLOCK_COUNT + 1) * BATCH_SEALING_BLOCK_COUNT


def _wait_for_snark_update_seq_no(
    log_path: Path,
    target_seq_no: int,
    after_offset: int,
    btc_rpc,
    miner_addr: str,
) -> int:
    def mine_and_find() -> list[int]:
        seq_nos = _submitted_update_seq_nos(log_path, after_offset)
        if target_seq_no not in seq_nos:
            # The EE proof/update pipeline advances after DA and OL state move
            # forward. Mining L1 blocks here drives those background services
            # while keeping the polling assertion local to this test.
            btc_rpc.proxy.generatetoaddress(4, miner_addr)
        return seq_nos

    seq_nos = wait_until_with_value(
        mine_and_find,
        lambda observed: target_seq_no in observed,
        error_with=(
            f"no snark update for seq_no={target_seq_no} within {SIGNAL_TIMEOUT_SECS}s "
            f"(observed={_submitted_update_seq_nos(log_path, after_offset)}, log={log_path})"
        ),
        timeout=SIGNAL_TIMEOUT_SECS,
        step=1.0,
    )
    logger.info("observed snark update seq_no=%s in %s", target_seq_no, seq_nos)
    return target_seq_no


def _send_padding_transfers_until_block(
    rpc,
    dev_account,
    gas_price: int,
    target_block: int,
) -> None:
    latest_block = int(rpc.eth_blockNumber(), 16)
    while latest_block < target_block:
        raw_tx = dev_account.sign_transfer(
            to=dev_account.address,
            value=0,
            gas_price=gas_price,
        )
        tx_hash = send_raw_transaction(rpc, raw_tx)
        receipt = wait_for_receipt(rpc, tx_hash, timeout=120)
        assert receipt["status"] == "0x1", f"padding transfer failed: {receipt}"
        latest_block = int(receipt["blockNumber"], 16)


@flexitest.register
class TestOsakaClz(BaseTest):
    """Exercise OP_CLZ in an EE block and prove it through the OL update path.

    Functional tests run the EE chunk/acct provers with the native host, so this
    does not produce a real SP1 proof. It does re-execute the EE block through
    the proof-program path before the snark account update is submitted to OL,
    which is the consistency property this regression test needs.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLEnv(
                fullnode_count=0,
                pre_generate_blocks=110,
                batch_sealing_block_count=BATCH_SEALING_BLOCK_COUNT,
            )
        )

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        rpc = alpen_seq.create_rpc()
        btc_rpc = bitcoin.create_rpc()
        miner_addr = btc_rpc.proxy.getnewaddress()
        log_path = _ee_log_path(alpen_seq)

        dev_account = get_dev_account(rpc)
        gas_price = int(rpc.eth_gasPrice(), 16)

        runtime = _clz_runtime()
        log_offset = log_path.stat().st_size if log_path.exists() else 0

        # Deploy raw bytecode instead of using Solidity so the test depends only
        # on the EVM opcode table and the bundled chainspec activation.
        deploy_raw_tx = _sign_deploy(
            dev_account.get_nonce(),
            _creation_code(runtime),
            gas_price,
        )
        deploy_tx_hash = send_raw_transaction(rpc, deploy_raw_tx)
        deploy_receipt = wait_for_receipt(rpc, deploy_tx_hash, timeout=120)
        assert deploy_receipt["status"] == "0x1", f"CLZ contract deploy failed: {deploy_receipt}"

        contract_addr = deploy_receipt["contractAddress"]
        assert contract_addr, f"missing contract address in receipt: {deploy_receipt}"

        # Confirm the init code returned exactly the CLZ runtime we intended to
        # execute. This keeps deployment bugs distinct from opcode failures.
        deployed_code = bytes.fromhex(rpc.eth_getCode(contract_addr, "latest")[2:])
        assert deployed_code == runtime, "deployed CLZ runtime bytecode mismatch"

        # Calling the contract executes CLZ and writes the result to slot 0.
        # eth-account requires checksum addresses when signing transactions.
        call_raw_tx = dev_account.sign_transaction(
            to=to_checksum_address(contract_addr),
            value=0,
            data=b"",
            gas_price=gas_price,
            gas=120_000,
        )
        call_tx_hash = send_raw_transaction(rpc, call_raw_tx)
        call_receipt = wait_for_receipt(rpc, call_tx_hash, timeout=120)
        assert call_receipt["status"] == "0x1", f"CLZ call failed: {call_receipt}"
        clz_block_number = int(call_receipt["blockNumber"], 16)

        slot_zero = int(rpc.eth_getStorageAt(contract_addr, "0x" + "0" * 64, "latest"), 16)
        assert slot_zero == CLZ_EXPECTED_RESULT, (
            f"CLZ result storage mismatch: got {slot_zero}, expected {CLZ_EXPECTED_RESULT}"
        )

        # Batches are sealed every `BATCH_SEALING_BLOCK_COUNT` EE blocks, and
        # update seq_no is `batch_idx - 1`. Wait for the specific update whose
        # batch covers the CLZ call block rather than accepting any later OL
        # submission from prover backlog.
        target_seq_no = _update_seq_no_for_ee_block(clz_block_number)
        target_batch_end = _batch_end_block_number(clz_block_number)
        _send_padding_transfers_until_block(
            rpc,
            dev_account,
            gas_price,
            target_batch_end,
        )
        _wait_for_snark_update_seq_no(log_path, target_seq_no, log_offset, btc_rpc, miner_addr)

        logger.info(
            "OP_CLZ executed in EE block %s and proved in snark update seq_no=%s",
            clz_block_number,
            target_seq_no,
        )
        return True

"""Crafted EE DA envelope injection helpers for functional tests."""

import json
import logging
import subprocess
from dataclasses import dataclass
from typing import Any

from common.services import BitcoinService
from envconfigs.alpen_client import DEFAULT_DA_MAGIC_BYTES

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class CraftedRevealTx:
    """Reveal transaction metadata returned by `strata-test-cli`."""

    index: int
    txid: str
    wtxid: str
    hex: str
    broadcast: bool


@dataclass(frozen=True)
class CraftedEnvelope:
    """Crafted envelope transaction metadata returned by `strata-test-cli`."""

    commit_txid: str
    commit_wtxid: str
    commit_hex: str
    broadcast_reveal_txids: list[str]
    reveal_txs: list[CraftedRevealTx]


def post_ee_da_envelope(
    bitcoin: BitcoinService,
    *,
    chunks: list[bytes],
    magic_bytes: bytes = DEFAULT_DA_MAGIC_BYTES,
    reveal_count: int | None = None,
    mine_mode: str = "inline",
    fee_rate: int = 2,
    timeout: int = 60,
) -> CraftedEnvelope:
    """Broadcast a crafted chunked envelope via `strata-test-cli`.

    In `manual` mode the caller owns all mining and can broadcast returned raw
    reveal tx hex values later with `broadcast_raw_tx`.
    """
    if not chunks:
        raise ValueError("chunks must contain at least one entry")

    bitcoind_url = f"http://127.0.0.1:{bitcoin.props['rpc_port']}"
    cmd = [
        "strata-test-cli",
        "post-ee-da-envelope",
        "--bitcoind-url",
        bitcoind_url,
        "--rpc-user",
        bitcoin.props["rpc_user"],
        "--rpc-password",
        bitcoin.props["rpc_password"],
        "--wallet-name",
        bitcoin.props["walletname"],
        "--magic-bytes",
        magic_bytes.decode("ascii"),
        "--fee-rate",
        str(fee_rate),
        "--mine-mode",
        mine_mode,
    ]
    if reveal_count is not None:
        cmd.extend(["--reveal-count", str(reveal_count)])
    for chunk in chunks:
        cmd.extend(["--chunk-hex", chunk.hex()])

    logger.info("Running: %s", " ".join(cmd))
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(
            f"post-ee-da-envelope failed ({result.returncode}):\n"
            f"stderr={result.stderr.strip()}\nstdout={result.stdout.strip()}"
        )
    return _parse_crafted_envelope(json.loads(result.stdout))


def broadcast_raw_tx(bitcoin: BitcoinService, tx_hex: str) -> str:
    """Broadcast a raw transaction through the test bitcoind."""
    btc_rpc = bitcoin.create_rpc()
    return btc_rpc.proxy.sendrawtransaction(tx_hex)


def mine_blocks(bitcoin: BitcoinService, block_count: int = 1) -> list[str]:
    """Mine `block_count` regtest blocks and return their block hashes."""
    btc_rpc = bitcoin.create_rpc()
    mine_address = btc_rpc.proxy.getnewaddress()
    return btc_rpc.proxy.generatetoaddress(block_count, mine_address)


def mine_empty_blocks(bitcoin: BitcoinService, block_count: int = 1) -> list[str]:
    """Mine empty regtest blocks without selecting transactions from the mempool."""
    btc_rpc = bitcoin.create_rpc()
    block_hashes: list[str] = []
    for _ in range(block_count):
        mine_address = btc_rpc.proxy.getnewaddress()
        result = btc_rpc.proxy.generateblock(mine_address, [])
        if isinstance(result, dict):
            block_hashes.append(result["hash"])
        else:
            block_hashes.append(result)
    return block_hashes


def inject_da_window(bitcoin: BitcoinService, inject) -> tuple[int, int]:
    """Run `inject` and return the inclusive L1 block window it affected."""
    btc_rpc = bitcoin.create_rpc()
    start_height = btc_rpc.proxy.getblockcount() + 1
    inject()
    end_height = btc_rpc.proxy.getblockcount()
    return start_height, end_height


def _parse_crafted_envelope(raw: dict[str, Any]) -> CraftedEnvelope:
    return CraftedEnvelope(
        commit_txid=raw["commit_txid"],
        commit_wtxid=raw["commit_wtxid"],
        commit_hex=raw["commit_hex"],
        broadcast_reveal_txids=list(raw["broadcast_reveal_txids"]),
        reveal_txs=[
            CraftedRevealTx(
                index=int(reveal["index"]),
                txid=reveal["txid"],
                wtxid=reveal["wtxid"],
                hex=reveal["hex"],
                broadcast=bool(reveal["broadcast"]),
            )
            for reveal in raw["reveal_txs"]
        ],
    )

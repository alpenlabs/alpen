import contextlib
import json
import logging
import math
import os
import pty
import subprocess
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from threading import Thread
from typing import Any, TypeVar

import base58
from bitcoinlib.services.bitcoind import BitcoindClient

from factory.config import BitcoindConfig
from factory.seqrpc import JsonrpcClient, RpcError
from utils.constants import *

# RPC error codes from crates/rpc/types/src/errors.rs
RPC_ERROR_MISSING_ASM_STATE = -32619  # ASM state not found
RPC_ERROR_MISSING_BRIDGE_V1_SECTION = -32620  # BridgeV1 section not found in ASM state
RPC_ERROR_BRIDGE_V1_DECODE_ERROR = -32621  # Failed to decode BridgeV1 state


def generate_jwt_secret() -> str:
    return os.urandom(32).hex()


def generate_blocks(
    bitcoin_rpc: BitcoindClient,
    wait_dur,
    addr: str,
) -> Thread:
    thr = Thread(
        target=generate_task,
        args=(
            bitcoin_rpc,
            wait_dur,
            addr,
        ),
    )
    thr.start()
    return thr


def generate_task(rpc: BitcoindClient, wait_dur, addr):
    while True:
        time.sleep(wait_dur)
        try:
            rpc.proxy.generatetoaddress(1, addr)
        except Exception as ex:
            logging.warning(f"{ex} while generating to address {addr}")
            return


def generate_n_blocks(bitcoin_rpc: BitcoindClient, n: int):
    addr = bitcoin_rpc.proxy.getnewaddress()
    print(f"generating {n} blocks to address", addr)
    try:
        blk = bitcoin_rpc.proxy.generatetoaddress(n, addr)
        print(f"made blocks {blk}")
        return blk
    except Exception as ex:
        logging.warning(f"{ex} while generating address")
        return


def wait_until(
    fn: Callable[[], Any],
    error_with: str = "Timed out",
    timeout: int = 30,
    step: float = 0.5,
):
    """
    Wait until a function call returns truth value, given time step, and timeout.
    This function waits until function call returns truth value at the interval of 1 sec
    """
    for _ in range(math.ceil(timeout / step)):
        try:
            # Return if the predicate passes.  The predicate not passing is not
            # an error.
            if fn():
                return
        except Exception as e:
            ety = type(e)
            logging.warning(f"caught exception {ety}, will still wait for timeout: {e}")
        time.sleep(step)
    raise AssertionError(error_with)


T = TypeVar("T")


def wait_until_with_value(
    fn: Callable[..., T],
    predicate: Callable[[T], bool],
    error_with: str = "Timed out",
    timeout: int = 5,
    step: float = 0.5,
    debug=False,
) -> T:
    """
    Similar to `wait_until` but this returns the value of the function.
    This also takes another predicate which acts on the function value and returns a bool
    """
    for _ in range(math.ceil(timeout / step)):
        try:
            r = fn()
            # Return if the predicate passes.  The predicate not passing is not
            # an error.
            if debug:
                print("Waiting.. current value:", r)
            if predicate(r):
                return r
        except Exception as e:
            ety = type(e)
            logging.warning(f"caught exception {ety}, will still wait for timeout: {e}")

        time.sleep(step)
    raise AssertionError(error_with)


@dataclass
class ManualGenBlocksConfig:
    btcrpc: BitcoindClient
    finality_depth: int
    gen_addr: str


@dataclass
class RollupParamsSettings:
    block_time_sec: int
    epoch_slots: int
    genesis_trigger: int
    message_interval: int
    proof_timeout: int | None = None
    chain_config: str | None = None

    @classmethod
    def new_default(cls):
        return cls(
            block_time_sec=DEFAULT_BLOCK_TIME_SEC,
            epoch_slots=DEFAULT_EPOCH_SLOTS,
            genesis_trigger=DEFAULT_GENESIS_TRIGGER_HT,
            message_interval=DEFAULT_MESSAGE_INTERVAL_MSEC,
            proof_timeout=DEFAULT_PROOF_TIMEOUT,
        )

    def fast_batch(self):
        self.proof_timeout = 1
        return self

    def strict_mode(self):
        self.proof_timeout = None
        return self

    def with_chainconfig(self, chain_config: str):
        self.chain_config = chain_config
        return self


@dataclass
class ProverClientSettings:
    native_workers: int
    polling_interval: int
    enable_checkpoint_proving: bool
    max_retry_counter: int

    @staticmethod
    def new_default():
        return ProverClientSettings(
            native_workers=DEFAULT_PROVER_NATIVE_WORKERS,
            polling_interval=DEFAULT_PROVER_POLLING_INTERVAL,
            enable_checkpoint_proving=DEFAULT_PROVER_ENABLE_CHECKPOINT_PROVING,
            max_retry_counter=DEFAULT_PROVER_MAX_RETRY_COUNTER,
        )

    @staticmethod
    def new_with_proving():
        return ProverClientSettings(
            native_workers=DEFAULT_PROVER_NATIVE_WORKERS,
            polling_interval=DEFAULT_PROVER_POLLING_INTERVAL,
            enable_checkpoint_proving=True,
            max_retry_counter=DEFAULT_PROVER_MAX_RETRY_COUNTER,
        )


def check_nth_checkpoint_finalized(
    idx: int,
    seqrpc,
    prover_rpc,
    manual_gen: ManualGenBlocksConfig | None = None,
    proof_timeout: int | None = None,
    **kwargs,
):
    """
    This check expects nth checkpoint to be finalized.

    It used to do this in an indirect way that had to be done in lockstep with
    the client state, but it's more flexible now.

    Params:
        - idx: The index of checkpoint
        - seqrpc: The sequencer rpc
        - manual_gen: If we need to generate blocks manually
    """

    def _maybe_do_gen():
        if manual_gen:
            nblocks = manual_gen.finality_depth + 1
            logging.debug(f"generating {nblocks} L1 blocks to try to finalize")
            manual_gen.btcrpc.proxy.generatetoaddress(nblocks, manual_gen.gen_addr)

    def _check():
        cs = seqrpc.strata_clientStatus()
        l1_height = cs["tip_l1_block"]["height"]
        fin_epoch = cs["finalized_epoch"]
        ss = seqrpc.strata_syncStatus()
        cur_epoch = ss["cur_epoch"]
        chain_l1_height = ss["safe_l1_block"]["height"]
        logging.info(
            f"finalized epoch as of {l1_height}: {fin_epoch} (cur chain epoch {cur_epoch}, \
                last L1 {chain_l1_height})"
        )
        if fin_epoch is not None and fin_epoch["epoch"] >= idx:
            return True
        _maybe_do_gen()
        return False

    wait_until(_check, **kwargs)


def submit_checkpoint(
    idx: int, seqrpc, prover_rpc, manual_gen: ManualGenBlocksConfig | None = None
):
    """
    Submits checkpoint and if manual_gen, waits till it is present in l1
    """
    from utils.wait.prover import ProverWaiter

    last_published_txid = seqrpc.strata_l1status()["last_published_txid"]

    # Post checkpoint proof
    # NOTE: Since operating in timeout mode is supported, i.e. sequencer
    # will post empty proof if prover doesn't submit proofs in time.
    proof_keys = prover_rpc.dev_strata_proveCheckpoint(idx)
    proof_key = proof_keys[0]
    prover_waiter = ProverWaiter(prover_rpc, logging.Logger("submit_checkpoint"), timeout=30)
    prover_waiter.wait_for_proof_completion(proof_key)
    proof = prover_rpc.dev_strata_getProof(proof_key)

    seqrpc.strataadmin_submitCheckpointProof(idx, proof)

    # Wait a while for it to be posted to l1. This will happen when there
    # is a new published txid in l1status
    published_txid = wait_until_with_value(
        lambda: seqrpc.strata_l1status()["last_published_txid"],
        predicate=lambda v: v != last_published_txid,
        error_with="Proof was not published to bitcoin",
        timeout=5,
    )

    if manual_gen:
        manual_gen.btcrpc.proxy.generatetoaddress(1, manual_gen.gen_addr)

        # Check it is confirmed
        wait_until(
            lambda: manual_gen.btcrpc.proxy.gettransaction(published_txid)["confirmations"] > 0,
            timeout=5,
            error_with="Published envelope not confirmed",
        )


def check_submit_proof_fails_for_nonexistent_batch(seqrpc, nonexistent_batch: int):
    """
    Requires that submitting nonexistent batch proof fails
    """
    empty_proof_receipt = {"proof": [], "public_values": []}

    try:
        seqrpc.strataadmin_submitCheckpointProof(nonexistent_batch, empty_proof_receipt)
    except Exception as e:
        if hasattr(e, "code"):
            assert e.code == ERROR_CHECKPOINT_DOESNOT_EXIST
        else:
            print("Unexpected error occurred")
            raise e
    else:
        raise AssertionError("Expected rpc error")


def check_already_sent_proof(seqrpc, sent_batch: int):
    """
    Requires that submitting proof that was already sent fails
    """
    empty_proof_receipt = {"proof": [], "public_values": []}
    try:
        # Proof for checkpoint 0 is already sent
        seqrpc.strataadmin_submitCheckpointProof(sent_batch, empty_proof_receipt)
    except Exception as e:
        assert e.code == ERROR_PROOF_ALREADY_CREATED
    else:
        raise AssertionError("Expected rpc error")


def generate_seed_at(path: str):
    """Generates a seed file at specified path."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",  # Global option: must come before subcommand
        "genxpriv",
        "-f", path
    ]
    # fmt: on

    res = subprocess.run(cmd, stdout=subprocess.PIPE)
    res.check_returncode()


def generate_seqpubkey_from_seed(path: str) -> str:
    """Generates a sequencer pubkey from the seed at file path."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",  # Global option: must come before subcommand
        "genseqpubkey",
        "-f", path
    ]
    # fmt: on

    with open(path) as f:
        print(f"sequencer root privkey {f.read()}")

    res = subprocess.run(cmd, stdout=subprocess.PIPE)
    res.check_returncode()
    res = str(res.stdout, "utf8").strip()
    assert len(res) > 0, "no output generated"
    print(f"SEQ PUBKEY {res}")
    return res


def generate_params(
    settings: RollupParamsSettings,
    seqpubkey: str,
    opxprivs: list[str],
    bitcoind_config: BitcoindConfig,
) -> str:
    """Generates a params file from config values."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",  # Global option: must come before subcommand
    ]

    # Add Bitcoin RPC configuration
    cmd.extend([
        "--bitcoin-rpc-url", bitcoind_config.rpc_url,
        "--bitcoin-rpc-user", bitcoind_config.rpc_user,
        "--bitcoin-rpc-password", bitcoind_config.rpc_password,
    ])

    cmd.extend([
        "genparams",
        "--name", "ALPN",
        "--block-time", str(settings.block_time_sec),
        "--epoch-slots", str(settings.epoch_slots),
        "--genesis-l1-height", str(settings.genesis_trigger),
        "--seqkey", seqpubkey,
    ])

    if settings.proof_timeout is not None:
        cmd.extend(["--proof-timeout", str(settings.proof_timeout)])

    if settings.chain_config is not None:
        cmd.extend(["--chain-config", settings.chain_config])
    # fmt: on

    for k in opxprivs:
        cmd.extend(["--opkey", k])

    res = subprocess.run(cmd, stdout=subprocess.PIPE)
    res.check_returncode()
    res = str(res.stdout, "utf8").strip()
    assert len(res) > 0, "no output generated"
    return res


def generate_asm_params(
    opxprivs: list[str],
    bitcoind_config: BitcoindConfig,
    genesis_l1_height: int,
) -> str:
    """Generates ASM params JSON from config values."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",
    ]

    cmd.extend([
        "--bitcoin-rpc-url", bitcoind_config.rpc_url,
        "--bitcoin-rpc-user", bitcoind_config.rpc_user,
        "--bitcoin-rpc-password", bitcoind_config.rpc_password,
    ])

    cmd.extend([
        "gen-asm-params",
        "--name", "ALPN",
        "--genesis-l1-height", str(genesis_l1_height),
    ])
    # fmt: on

    for k in opxprivs:
        cmd.extend(["--opkey", k])

    res = subprocess.run(cmd, stdout=subprocess.PIPE)
    res.check_returncode()
    res = str(res.stdout, "utf8").strip()
    assert len(res) > 0, "no output generated"
    return res


def generate_simple_params(
    base_path: str,
    settings: RollupParamsSettings,
    operator_cnt: int,
    bitcoind_config: BitcoindConfig,
) -> dict:
    """
    Creates a network with params data and a list of operator seed paths.

    If bitcoind_config is provided, will fetch the L1 block hash from Bitcoin RPC.

    Result options are `params`, `asm_params`, and `opseedpaths`.
    """
    seqseedpath = os.path.join(base_path, "seqkey.bin")
    opseedpaths = [os.path.join(base_path, "opkey%s.bin") % i for i in range(operator_cnt)]
    for p in [seqseedpath] + opseedpaths:
        generate_seed_at(p)

    seqkey = generate_seqpubkey_from_seed(seqseedpath)
    opxprivs = []
    for p in opseedpaths:
        with open(p) as f:
            opxprivs.append(f.read().strip())

    params = generate_params(settings, seqkey, opxprivs, bitcoind_config)
    print(f"Params {params}")

    asm_params = generate_asm_params(opxprivs, bitcoind_config, settings.genesis_trigger)
    print(f"ASM Params {asm_params}")

    return {"params": params, "asm_params": asm_params, "opseedpaths": opseedpaths}


def broadcast_tx(btcrpc: BitcoindClient, outputs: list[dict], options: dict) -> str:
    """
    Broadcast a transaction to the Bitcoin network.
    """
    psbt_result = btcrpc.proxy.walletcreatefundedpsbt([], outputs, 0, options)
    psbt = psbt_result["psbt"]

    signed_psbt = btcrpc.proxy.walletprocesspsbt(psbt)

    finalized_psbt = btcrpc.proxy.finalizepsbt(signed_psbt["psbt"])
    deposit_tx = finalized_psbt["hex"]

    txid = btcrpc.sendrawtransaction(deposit_tx).get("txid", "")

    return txid


def get_bridge_pubkey(seqrpc) -> str:
    """
    Get the bridge pubkey from the sequencer.
    """
    from factory.test_cli import convert_to_xonly_pk, musig_aggregate_pks

    # Wait until genesis
    wait_until(
        lambda: seqrpc.strata_syncStatus() is not None,
        error_with="Genesis did not happen in time",
    )
    op_pks = seqrpc.strata_getActiveOperatorChainPubkeySet()
    print(f"Operator pubkeys: {op_pks}")
    # This returns a dict with index as key and pubkey as value
    # Iterate all of them ant then call musig_aggregate_pks
    # Also since they are full pubkeys, we need to convert them
    # to X-only pubkeys.
    op_pks = [op_pks[str(i)] for i in range(len(op_pks))]
    op_x_only_pks = [convert_to_xonly_pk(pk) for pk in op_pks]
    agg_pubkey = musig_aggregate_pks(op_x_only_pks)
    return agg_pubkey


def get_bridge_pubkey_from_cfg(cfg_params) -> str:
    """
    Get the bridge pubkey from the config.
    """
    from factory.test_cli import convert_to_xonly_pk, musig_aggregate_pks

    # Slight hack to convert to appropriate operator pubkey from cfg values.
    op_pks = ["02" + pk for pk in cfg_params.operators]
    op_x_only_pks = [convert_to_xonly_pk(pk) for pk in op_pks]
    agg_pubkey = musig_aggregate_pks(op_x_only_pks)
    return agg_pubkey


def setup_root_logger():
    """
    reads `LOG_LEVEL` from the environment. Defaults to `WARNING` if not provided.
    """
    log_level = os.getenv("LOG_LEVEL", "INFO").upper()
    log_level = getattr(logging, log_level, logging.NOTSET)
    # Configure the root logger
    root_logger = logging.getLogger()
    root_logger.setLevel(log_level)


def setup_test_logger(datadir_root: str, test_name: str) -> logging.Logger:
    """
    Set up logger for a given test, with corresponding log file in a logs directory.
    - Configures both file and stream handlers for the test logger.
    - Logs are stored in `<datadir_root>/logs/<test_name>.log`.

    Parameters:
        datadir_root (str): Root directory for logs.
        test_name (str): A test names to create loggers for.

    Returns:
        logging.Logger
    """
    # Create the logs directory
    log_dir = os.path.join(datadir_root, "logs")
    os.makedirs(log_dir, exist_ok=True)

    # Common formatter
    formatter = logging.Formatter(
        "%(asctime)s - %(levelname)s - %(filename)s:%(lineno)d - %(message)s"
    )

    # Set up individual loggers for each test
    logger = logging.getLogger(f"root.{test_name}")

    # File handler
    log_path = os.path.join(log_dir, f"{test_name}.log")
    file_handler = logging.FileHandler(log_path)
    file_handler.setFormatter(formatter)

    # Stream handler
    stream_handler = logging.StreamHandler()
    stream_handler.setFormatter(formatter)

    # Add handlers to the logger
    logger.addHandler(file_handler)
    logger.addHandler(stream_handler)

    # Set level to something sensible.
    log_level = os.getenv("LOG_LEVEL", "INFO").upper()
    logger.setLevel(log_level)

    return logger


def setup_load_job_logger(datadir_root: str, job_name: str):
    """
    Set up loggers for a given load job.
    - Configures file handlers for the test logger.
    - Logs are stored in `<datadir_root>/<env>/<load_service_name>/<job_name>.log`.

    Parameters:
        datadir_root (str): Root directory for logs.
        test_name (str): A load job name to create loggers for.

    Returns:
        logging.Logger
    """
    # Common formatter
    # We intentionally skip filename:line_number because most of the logs are coming
    # from the same place - logging transactions when sent, logging blocks when received, etc.
    formatter = logging.Formatter("%(asctime)s - %(levelname)s - %(message)s")
    # Set up individual loggers for each load job.
    filename = os.path.join(datadir_root, f"{job_name}.log")
    logger = logging.getLogger(job_name)

    # File handler
    file_handler = logging.FileHandler(filename)
    file_handler.setFormatter(formatter)

    # Add file handler to the logger
    logger.addHandler(file_handler)

    return logger


def get_envelope_pushdata(inp: str):
    op_if = "63"
    op_endif = "68"
    op_pushbytes_33 = "21"
    op_false = "00"
    start_position = inp.index(f"{op_false}{op_if}")
    end_position = inp.index(f"{op_endif}{op_pushbytes_33}", start_position)
    op_if_block = inp[start_position + 3 : end_position]
    op_pushdata = "4d"
    pushdata_position = op_if_block.index(f"{op_pushdata}")
    # we don't want PUSHDATA + num bytes b401
    return op_if_block[pushdata_position + 2 + 4 :]


def submit_da_blob(btcrpc: BitcoindClient, seqrpc: JsonrpcClient, blobdata: str):
    _ = seqrpc.strataadmin_submitDABlob(blobdata)

    # if blob data is present in tx witness then return the transaction
    tx = wait_until_with_value(
        lambda: btcrpc.gettransaction(seqrpc.strata_l1status()["last_published_txid"]),
        predicate=lambda tx: blobdata in tx.witness_data().hex(),
        timeout=10,
    )
    return tx


def cl_slot_to_block_id(seqrpc, slot):
    """Convert L2 slot number to block ID."""
    l2_blocks = seqrpc.strata_getHeadersAtIdx(slot)
    return l2_blocks[0]["block_id"]


def el_slot_to_block_commitment(rethrpc, block_num):
    """Get EL block commitment from block number using Ethereum RPC."""
    blk_id = rethrpc.eth_getBlockByNumber(hex(block_num), False)["hash"]
    return (block_num, blk_id)


def bytes_to_big_endian(hash):
    """Reverses the byte order of a hexadecimal string to produce big-endian format."""
    return "".join(reversed([hash[i : i + 2] for i in range(0, len(hash), 2)]))


def check_sequencer_down(seqrpc):
    """
    Returns True if sequencer RPC is down
    """
    try:
        seqrpc.strata_protocolVersion()
        return False
    except RuntimeError:
        return True


def confirm_btc_withdrawal(
    svc,
    original_balance,
    expected_increase,
    debug_fn=print,
):
    """
    Wait for the BTC balance to reflect the withdrawal and confirm the final balance
    equals `original_balance + expected_increase`.
    """
    # Wait for the new balance,
    # this includes waiting for a new batch checkpoint,
    # duty processing by the bridge clients and maturity of the withdrawal.
    wait_until(
        lambda: svc.l1_balance() > original_balance,
        timeout=60,
    )

    # Check final BTC balance
    btc_balance = svc.l1_balance()
    debug_fn(f"BTC final balance: {btc_balance}")
    debug_fn(f"Expected final balance: {original_balance + expected_increase}")

    assert btc_balance == original_balance + expected_increase, (
        "BTC balance after withdrawal is not as expected"
    )


def get_latest_eth_block_number(reth_rpc) -> int:
    """Get the current block number from reth RPC."""
    return int(reth_rpc.eth_blockNumber(), base=16)


def check_initial_eth_balance(rethrpc, address, debug_fn=print):
    """Asserts that the initial ETH balance for `address` is zero."""
    balance = int(rethrpc.eth_getBalance(address), 16)
    debug_fn(f"Strata Balance before deposits: {balance}")
    assert balance == 0, "Strata balance is not expected (should be zero initially)"


def get_priv_keys(ctx, env=None):
    if env is None:
        path = os.path.join(ctx.datadir_root, f"_{ctx.name}", "_init")
    else:
        path = os.path.join(ctx.datadir_root, env, "_init")

    priv_keys = []
    opkeys = sorted(
        filter(lambda file: file.startswith("opkey"), os.listdir(path)),
        key=lambda x: int("".join(filter(str.isdigit, x))),
    )
    for filename in opkeys:
        if not filename.startswith("op"):
            continue

        full_path = os.path.join(path, filename)
        with open(full_path) as f:
            content = f.read().strip()
            decoded = base58.b58decode(content)[:-4]  # remove checksum
            priv_keys.append(decoded)
    return priv_keys


def run_tty(cmd, *, capture_output=False, stdout=None, env=None) -> subprocess.CompletedProcess:
    """
    Runs `cmd` under a PTY (so indicatif used by Alpen-cli behaves).
    Returns a CompletedProcess; stdout is bytes when captured.
    """
    if stdout is subprocess.PIPE:
        capture_output, stdout = True, None

    buf = [] if capture_output else None

    # Create a pseudo-terminal pair
    master_fd, slave_fd = pty.openpty()

    try:
        # Prepare environment - inherit current env and merge with custom env
        proc_env = os.environ.copy()
        if env:
            proc_env.update(env)

        # Start subprocess with the slave side of the PTY
        proc = subprocess.Popen(
            cmd,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            env=proc_env,
            close_fds=True,
        )

        # Close slave_fd in parent process (child has its own copy)
        os.close(slave_fd)
        slave_fd = -1  # Mark as closed

        # Read from master_fd until process completes
        while True:
            try:
                data = os.read(master_fd, 4096)
                if not data:
                    break

                if buf is not None:
                    buf.append(data)
                elif stdout is None:
                    os.write(1, data)  # parent stdout
                else:
                    # file-like or text stream
                    if hasattr(stdout, "buffer"):
                        stdout.buffer.write(data)
                        stdout.flush()
                    else:
                        stdout.write(data.decode("utf-8", "replace"))
                        if hasattr(stdout, "flush"):
                            stdout.flush()
            except OSError:
                # PTY closed, process likely finished
                break

        # Wait for process to complete
        rc = proc.wait()

    finally:
        # Clean up file descriptors
        if slave_fd != -1:
            with contextlib.suppress(OSError):
                os.close(slave_fd)
        with contextlib.suppress(OSError):
            os.close(master_fd)

    return subprocess.CompletedProcess(
        args=cmd,
        returncode=rc,
        stdout=(b"".join(buf) if buf is not None else None),
        stderr=None,  # PTY merges stderr
    )


def _ensure_solc_installed():
    """Ensure solc version is installed (auto-install on first use)."""
    result = subprocess.run(
        ["solc-select", "use", SOLC_VERSION],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        # Version not installed, install it
        subprocess.run(
            ["solc-select", "install", SOLC_VERSION],
            capture_output=True,
            text=True,
            check=True,
        )
        subprocess.run(
            ["solc-select", "use", SOLC_VERSION],
            capture_output=True,
            text=True,
            check=True,
        )


def compile_solidity(source: str, contract_name: str | None = None) -> tuple[list, str]:
    """
    Compile Solidity source and return (abi, bytecode).

    Auto-installs solc 0.8.24 on first use.

    Args:
        source: File path (e.g., "contracts/Counter.sol") or inline source code
        contract_name: Contract to extract. If None, extracts the only contract.

    Returns:
        Tuple of (abi, bytecode_hex)
    """
    _ensure_solc_installed()

    # Determine if source is file path or inline code
    is_file = Path(source).exists()

    if is_file:
        temp_file = None
        source_file = source
    else:
        # Write inline source to temp file
        with tempfile.NamedTemporaryFile(mode="w", suffix=".sol", delete=False) as f:
            f.write(source)
            temp_file = f.name
            source_file = temp_file

    try:
        result = subprocess.run(
            ["solc", "--combined-json", "abi,bin", source_file],
            capture_output=True,
            text=True,
            check=True,
        )

        contracts = json.loads(result.stdout)["contracts"]

        # Find contract by name or get the only one
        if contract_name:
            for contract_id, data in contracts.items():
                if contract_id.endswith(f":{contract_name}"):
                    abi_data = data["abi"]
                    abi = json.loads(abi_data) if isinstance(abi_data, str) else abi_data
                    return abi, data["bin"]
            raise RuntimeError(f"Contract '{contract_name}' not found")

        if len(contracts) != 1:
            contract_names = list(contracts.keys())
            raise RuntimeError(f"Expected 1 contract, found {len(contracts)}: {contract_names}")

        data = next(iter(contracts.values()))
        abi_data = data["abi"]
        abi = json.loads(abi_data) if isinstance(abi_data, str) else abi_data
        return abi, data["bin"]

    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"Compilation failed: {e.stderr}") from e
    finally:
        if temp_file:
            Path(temp_file).unlink(missing_ok=True)


def retry_rpc_with_asm_backoff(
    rpc_fn: Callable[[], T],
    timeout: int = 30,
    step: float = 1.0,
) -> T:
    """
    Retry an RPC call with backoff when ASM state is not yet available.

    This helper function wraps RPC calls that depend on ASM state being ready.
    It will retry the call if it fails with an ASM-related error, allowing
    time for the ASM worker to process L1 blocks and build state.

    Args:
        rpc_fn: The RPC function to call
        timeout: Maximum time to retry (seconds)
        step: Time between retries (seconds)

    Returns:
        The result of the RPC call

    Raises:
        AssertionError: If the timeout is reached without success
        Exception: Any non-ASM-related exceptions from the RPC call
    """

    def predicate():
        try:
            return rpc_fn()
        except RpcError as e:
            # Check if this is an ASM-not-ready error using specific error codes
            if e.code in (
                RPC_ERROR_MISSING_ASM_STATE,
                RPC_ERROR_MISSING_BRIDGE_V1_SECTION,
                RPC_ERROR_BRIDGE_V1_DECODE_ERROR,
            ):
                logging.debug(f"ASM not ready (error code {e.code}), will retry: {e}")
                return None  # Signal to keep retrying
            # Re-raise if it's a different error
            raise
        except Exception:
            # Re-raise non-RPC errors
            raise

    return wait_until_with_value(
        predicate,
        lambda v: v is not None,
        timeout=timeout,
        step=step,
        error_with="Timeout waiting for ASM state to be available for RPC call",
    )

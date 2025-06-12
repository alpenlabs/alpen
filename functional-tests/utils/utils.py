import logging
import math
import os
import subprocess
import time
from dataclasses import dataclass
from threading import Thread
from typing import Any, Callable, Optional, TypeVar

from bitcoinlib.services.bitcoind import BitcoindClient
from strata_utils import convert_to_xonly_pk, get_balance, musig_aggregate_pks

from factory.seqrpc import JsonrpcClient, RpcError
from utils.constants import *


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


def wait_for_genesis(rpc, timeout=20, step=2, **kwargs):
    """
    Waits until we see genesis.  That is to say, that `strata_syncStatus`
    returns a sensible result.
    """

    def _check_genesis():
        try:
            # This should raise if we're before genesis.
            ss = rpc.strata_syncStatus()
            logging.info(
                f"after genesis, tip is slot {ss['tip_height']} blkid {ss['tip_block_id']}"
            )
            return True
        except RpcError as e:
            # This is the "before genesis" error code, meaning we're still
            # before genesis
            if e.code == -32607:
                return False
            else:
                raise e

    wait_until(_check_genesis, timeout=timeout, step=step, **kwargs)


def wait_until_chain_epoch(rpc, epoch: int, **kwargs) -> dict:
    """
    Waits until the chain has finished the specified epoch index, determined by
    checking for epoch summaries.

    Returns the epoch summary.
    """

    logging.info(f"waiting for epoch {epoch}")

    def _query():
        status = rpc.strata_syncStatus()
        logging.debug(f"checked status {status}")
        commitments = rpc.strata_getEpochCommitments(epoch)
        if len(commitments) > 0:
            comm = commitments[0]
            logging.info(
                f"now at epoch {epoch}, slot {comm['last_slot']}, blkid {comm['last_blkid']}"
            )
            return rpc.strata_getEpochSummary(epoch, comm["last_slot"], comm["last_blkid"])
        return None

    def _check(v):
        return v is not None

    return wait_until_with_value(_query, _check, **kwargs)


def wait_until_next_chain_epoch(rpc, **kwargs) -> int:
    """
    Waits until the chain epoch advances by at least 1.

    Returns the new epoch number.
    """
    init_epoch = rpc.strata_syncStatus()["cur_epoch"]

    def _query():
        ss = rpc.strata_syncStatus()
        logging.info(f"waiting for next epoch, ss {ss}")
        return ss["cur_epoch"]

    def _check(epoch):
        return epoch > init_epoch

    return wait_until_with_value(_query, _check, **kwargs)


def wait_until_epoch_confirmed(rpc, epoch: int, **kwargs):
    """
    Waits until at least the given epoch is confirmed on L1, according to
    calling `strata_clientStatus`.
    """

    def _check():
        cs = rpc.strata_clientStatus()
        l1_height = cs["tip_l1_block"]["height"]
        conf_epoch = cs["confirmed_epoch"]
        logging.info(f"confirmed epoch as of {l1_height}: {conf_epoch}")
        if conf_epoch is None:
            return False
        return conf_epoch["epoch"] >= epoch

    wait_until(_check, **kwargs)


def wait_until_epoch_finalized(rpc, epoch: int, **kwargs):
    """
    Waits until at least the given epoch is finalized on L1, according to
    calling `strata_clientStatus`.
    """

    def _check():
        cs = rpc.strata_clientStatus()
        l1_height = cs["tip_l1_block"]["height"]
        fin_epoch = cs["finalized_epoch"]
        logging.info(f"finalized epoch as of {l1_height}: {fin_epoch}")
        if fin_epoch is None:
            return False
        return fin_epoch["epoch"] >= epoch

    wait_until(_check, **kwargs)


def wait_until_epoch_observed_final(rpc, epoch: int, **kwargs):
    """
    Waits until at least the given epoch is observed as final on L2, according
    to calling `strata_syncStatus`.
    """

    def _check():
        ss = rpc.strata_syncStatus()
        slot = ss["tip_height"]  # TODO rename to tip_slot
        of_epoch = ss["observed_finalized_epoch"]
        logging.info(f"observed final epoch as of L2 slot {slot}: {of_epoch}")
        if not of_epoch:
            return False
        return of_epoch["epoch"] >= epoch

    wait_until(_check, **kwargs)


def wait_until_l1_observed(rpc, height: int, **kwargs):
    """
    Waits until the provided L1 height has been observed by the chain.
    """

    def _check():
        ss = rpc.strata_syncStatus()
        slot = ss["tip_height"]  # TODO rename to slot
        epoch = ss["cur_epoch"]
        view_l1 = ss["safe_l1_block"]["height"]
        logging.info(f"chain now at slot {slot}, epoch {epoch}, observed L1 height is {view_l1}")
        return view_l1 >= height

    wait_until(_check, **kwargs)


def wait_until_csm_l1_tip_observed(rpc, **kwargs):
    """
    Waits until the CSM's current L1 tip block height has been observed by the OL.
    """

    init_cs = rpc.strata_clientStatus()
    init_l1_height = init_cs["tip_l1_block"]["height"]
    logging.info(f"target L1 height from CSM is {init_l1_height}")
    wait_until_l1_observed(init_l1_height, **kwargs)


def wait_until_cur_l1_tip_observed(btcrpc, seqrpc, **kwargs):
    """
    Waits until the current L1 tip block as requested from the L1 RPC has been
    observed by the CSM.

    Returns the L1 block height.
    """
    info = btcrpc.proxy.getblockchaininfo()
    h = info["blocks"]
    logging.info(f"current bitcoin height is {h}")
    wait_until_l1_observed(seqrpc, h, **kwargs)


@dataclass
class ManualGenBlocksConfig:
    btcrpc: BitcoindClient
    finality_depth: int
    gen_addr: str


@dataclass
class RollupParamsSettings:
    block_time_sec: int
    epoch_slots: int
    horizon_height: int
    genesis_trigger: int
    message_interval: int
    proof_timeout: Optional[int] = None
    chain_config: Optional[str] = None

    @classmethod
    def new_default(cls):
        return cls(
            block_time_sec=DEFAULT_BLOCK_TIME_SEC,
            epoch_slots=DEFAULT_EPOCH_SLOTS,
            horizon_height=DEFAULT_HORIZON_HT,
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
    last_published_txid = seqrpc.strata_l1status()["last_published_txid"]

    # Post checkpoint proof
    # NOTE: Since operating in timeout mode is supported, i.e. sequencer
    # will post empty proof if prover doesn't submit proofs in time.
    proof_keys = prover_rpc.dev_strata_proveCheckpoint(idx)
    proof_key = proof_keys[0]
    wait_for_proof_with_time_out(prover_rpc, proof_key)
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


def wait_for_proof_with_time_out(prover_client_rpc, task_id, time_out=3600) -> bool:
    """
    Waits for a proof task to complete/fail within a specified timeout period.

    This function continuously polls the status of a proof task identified by `task_id` using
    the `prover_client_rpc` client. It checks the status every 2 seconds and waits until the
    proof task status is either "Completed" where it returns True, or "Failed" where it return
    False. If the specified `time_out` (in seconds) is reached, it throws TimeoutError.
    """

    start_time = time.time()
    while True:
        # Fetch the proof status
        proof_status = prover_client_rpc.dev_strata_getTaskStatus(task_id)
        assert proof_status is not None
        logging.info(f"Got the proof status {proof_status}")
        if proof_status == "Completed":
            logging.info(f"Completed the proof generation for {task_id}")
            return True
        elif proof_status == "Failed":
            logging.info(f"Proof generatoin failed for {task_id}")
            return False

        time.sleep(2)
        elapsed_time = time.time() - start_time  # Calculate elapsed time
        if elapsed_time >= time_out:
            raise TimeoutError(f"Operation timed out after {time_out} seconds.")


def generate_seed_at(path: str):
    """Generates a seed file at specified path."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",
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
        "-b", "regtest",
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


def generate_opxpub_from_seed(path: str) -> str:
    """Generates operate pubkey from seed at file path."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",
        "genopxpub",
        "-f", path
    ]
    # fmt: on

    res = subprocess.run(cmd, stdout=subprocess.PIPE)
    res.check_returncode()
    res = str(res.stdout, "utf8").strip()
    assert len(res) > 0, "no output generated"
    return res


def generate_params(settings: RollupParamsSettings, seqpubkey: str, oppubkeys: list[str]) -> str:
    """Generates a params file from config values."""
    # fmt: off
    cmd = [
        "strata-datatool",
        "-b", "regtest",
        "genparams",
        "--name", "ALPN",
        "--block-time", str(settings.block_time_sec),
        "--epoch-slots", str(settings.epoch_slots),
        "--horizon-height", str(settings.horizon_height),
        "--genesis-trigger-height", str(settings.genesis_trigger),
        "--seqkey", seqpubkey,
    ]
    if settings.proof_timeout is not None:
        cmd.extend(["--proof-timeout", str(settings.proof_timeout)])

    if settings.chain_config is not None:
        cmd.extend(["--chain-config", settings.chain_config])
    # fmt: on

    for k in oppubkeys:
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
) -> dict:
    """
    Creates a network with params data and a list of operator seed paths.

    Result options are `params` and `opseedpaths`.
    """
    seqseedpath = os.path.join(base_path, "seqkey.bin")
    opseedpaths = [os.path.join(base_path, "opkey%s.bin") % i for i in range(operator_cnt)]
    for p in [seqseedpath] + opseedpaths:
        generate_seed_at(p)

    seqkey = generate_seqpubkey_from_seed(seqseedpath)
    opxpubs = [generate_opxpub_from_seed(p) for p in opseedpaths]

    params = generate_params(settings, seqkey, opxpubs)
    print(f"Params {params}")
    return {"params": params, "opseedpaths": opseedpaths}


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
    # Slight hack to convert to appropriate operator pubkey from cfg values.
    op_pks = ["02" + pk for pk in cfg_params.operator_config.get_operators_pubkeys()]
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
    withdraw_address,
    btc_url,
    btc_user,
    btc_password,
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
        lambda: get_balance(withdraw_address, btc_url, btc_user, btc_password) > original_balance,
        timeout=60,
    )

    # Check final BTC balance
    btc_balance = get_balance(withdraw_address, btc_url, btc_user, btc_password)
    debug_fn(f"BTC final balance: {btc_balance}")
    debug_fn(f"Expected final balance: {original_balance + expected_increase}")

    assert btc_balance == original_balance + expected_increase, (
        "BTC balance after withdrawal is not as expected"
    )


def check_initial_eth_balance(rethrpc, address, debug_fn=print):
    """Asserts that the initial ETH balance for `address` is zero."""
    balance = int(rethrpc.eth_getBalance(address), 16)
    debug_fn(f"Strata Balance before deposits: {balance}")
    assert balance == 0, "Strata balance is not expected (should be zero initially)"

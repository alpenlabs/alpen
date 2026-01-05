import copy
import json
import logging
import time

import flexitest

from envs.rollup_params_cfg import RollupConfig
from factory.alpen_client import AlpenCliBuilder
from factory.config import BitcoindConfig, RethELConfig
from factory.test_cli import get_address
from load.cfg import LoadConfig, LoadConfigBuilder
from utils import *
from utils.constants import *
from utils.wait import ProverWaiter, RethWaiter, StrataWaiter


def _retry_bitcoind_rpc(
    logger: logging.Logger,
    desc: str,
    fn,
    *,
    timeout_s: float = 180.0,
    interval_s: float = 1.0,
):
    """
    Best-effort retry wrapper for bitcoind JSON-RPC calls.

    We use it during env init to reduce flakiness when bitcoind is still warming up
    (especially when CI runs each test in its own fresh runner).
    """
    deadline = time.monotonic() + timeout_s
    last_exc: Exception | None = None

    while time.monotonic() < deadline:
        try:
            return fn()
        except Exception as e:  # noqa: BLE001 - broad retry is intentional for test harness
            last_exc = e
            logger.info(f"bitcoind RPC '{desc}' failed; retrying in {interval_s:.1f}s: {e!r}")
            time.sleep(interval_s)

    raise RuntimeError(
        f"bitcoind RPC '{desc}' did not succeed within {timeout_s:.0f}s"
    ) from last_exc


class StrataTestBase(flexitest.Test):
    """
    Class to be used instead of flexitest.Test for accessing logger
    """

    def premain(self, ctx: flexitest.RunContext):
        logger = setup_test_logger(ctx.datadir_root, ctx.name)
        self.logger = logger
        self.debug = logger.debug
        self.info = logger.info
        self.warning = logger.warning
        self.error = logger.error
        self.critical = logger.critical

    def create_reth_waiter(self, reth_rpc, timeout=20, interval=0.5, **kwargs) -> RethWaiter:
        return RethWaiter(reth_rpc, self.logger, timeout, interval, **kwargs)

    def create_strata_waiter(self, strata_rpc, timeout=20, interval=0.5, **kwargs) -> StrataWaiter:
        return StrataWaiter(strata_rpc, self.logger, timeout, interval, **kwargs)

    def create_prover_waiter(self, prover_rpc, timeout=20, interval=0.5, **kwargs) -> ProverWaiter:
        return ProverWaiter(prover_rpc, self.logger, timeout, interval, **kwargs)


class StrataTestRuntime(flexitest.TestRuntime):
    """
    Extended testenv.StrataTestRuntime to call custom run context
    """

    def create_run_context(self, name: str, env: flexitest.LiveEnv) -> flexitest.RunContext:
        return StrataRunContext(self.datadir_root, name, env)


class StrataRunContext(flexitest.RunContext):
    """
    Custom run context which provides access to services and some test specific variables.
    To be used by ExtendedTestRuntime
    """

    def __init__(self, datadir_root: str, name: str, env: flexitest.LiveEnv):
        super().__init__(env)
        self.name = name
        self.datadir_root = datadir_root

        rollup_cfg = env.rollup_cfg()
        agg_pubkey = get_bridge_pubkey_from_cfg(rollup_cfg)
        rollup_params_json = env.rollup_params_json()

        builder = (
            AlpenCliBuilder()
            .requires_service(
                "bitcoin",
                lambda s: BitcoindConfig(
                    rpc_url=f"http://localhost:{s.get_prop('rpc_port')}",
                    rpc_user=s.get_prop("rpc_user"),
                    rpc_password=s.get_prop("rpc_password"),
                ),
            )
            .requires_service(
                "reth",
                lambda s: f"http://localhost:{s.get_prop('eth_rpc_http_port')}",
            )
            .with_pubkey(agg_pubkey)
            .with_magic_bytes(rollup_cfg.magic_bytes)
            .with_datadir(os.path.join(self.datadir_root, name))
        )

        if rollup_params_json:
            builder = builder.with_rollup_params(rollup_params_json)

        self.alpen_cli = builder.build(self)


class BasicLiveEnv(flexitest.LiveEnv):
    """
    A common thin layer for all instances of the Environments.
    """

    def __init__(self, srvs, bridge_pk, rollup_cfg: RollupConfig, rollup_params_json: str = None):
        super().__init__(srvs)
        self._el_address_gen = (
            f"deada00{x:04X}dca3ebeefdeadf001900dca3ebeef" for x in range(16**4)
        )
        self._ext_btc_addr_idx = 0
        self._rec_btc_addr_idx = 0
        self._bridge_pk = bridge_pk
        self._rollup_cfg = rollup_cfg
        self._rollup_params_json = rollup_params_json

    def gen_el_address(self) -> str:
        """
        Generates a unique EL address to be used across tests.
        """
        return next(self._el_address_gen)

    def gen_ext_btc_address(self) -> str | list[str]:
        """
        Generates a unique bitcoin (external) taproot addresses that is funded with some BTC.
        """

        tr_addr: str = get_address(self._ext_btc_addr_idx)
        self._ext_btc_addr_idx += 1
        return tr_addr

    def rollup_cfg(self) -> RollupConfig:
        return self._rollup_cfg

    def rollup_params_json(self) -> str | None:
        """Returns the rollup params as a JSON string"""
        return self._rollup_params_json


class BasicEnvConfig(flexitest.EnvConfig):
    def __init__(
        self,
        pre_generate_blocks: int = 0,
        rollup_settings: RollupParamsSettings | None = None,
        prover_client_settings: ProverClientSettings | None = None,
        auto_generate_blocks: bool = True,
        pre_fund_addrs: bool = True,
        n_operators: int = 2,
        message_interval: int = 0,
        duty_timeout_duration: int = 10,
        custom_chain: str | dict = "dev",
        epoch_gas_limit: int | None = None,
        enable_state_diff_gen: bool = False,
    ):
        super().__init__()
        self.pre_generate_blocks = pre_generate_blocks
        self.rollup_settings = rollup_settings
        self.prover_client_settings = prover_client_settings
        self.auto_generate_blocks = auto_generate_blocks
        self.pre_fund_addrs = pre_fund_addrs
        self.n_operators = n_operators
        self.message_interval = message_interval
        self.duty_timeout_duration = duty_timeout_duration
        self.custom_chain = custom_chain
        self.epoch_gas_limit = epoch_gas_limit
        self.enable_state_diff_gen = enable_state_diff_gen

    def init(self, ctx: flexitest.EnvContext) -> flexitest.LiveEnv:
        btc_fac = ctx.get_factory("bitcoin")
        seq_fac = ctx.get_factory("sequencer")
        seq_signer_fac = ctx.get_factory("sequencer_signer")
        reth_fac = ctx.get_factory("reth")

        logger = logging.getLogger(__name__)

        svcs = {}

        # set up network params
        initdir = ctx.make_service_dir("_init")

        custom_chain = self.custom_chain
        if isinstance(custom_chain, dict):
            json_path = os.path.join(initdir, "custom_chain.json")
            with open(json_path, "w") as f:
                json.dump(custom_chain, f)
            custom_chain = json_path

        settings = (
            self.rollup_settings or RollupParamsSettings.new_default().fast_batch().strict_mode()
        )
        if custom_chain != self.custom_chain:
            settings = settings.with_chainconfig(custom_chain)

        # reth needs some time to startup, start it first
        secret_dir = ctx.make_service_dir("secret")
        reth_secret_path = os.path.join(secret_dir, "jwt.hex")

        with open(reth_secret_path, "w") as f:
            f.write(generate_jwt_secret())

        reth = reth_fac.create_exec_client(
            0,
            reth_secret_path,
            None,
            custom_chain=custom_chain,
            enable_state_diff_gen=self.enable_state_diff_gen,
        )
        reth_port = reth.get_prop("rpc_port")

        bitcoind = btc_fac.create_regtest_bitcoin()
        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )
        svcs["bitcoin"] = bitcoind
        time.sleep(BLOCK_GENERATION_INTERVAL_SECS)

        brpc = _retry_bitcoind_rpc(logger, "create_rpc", bitcoind.create_rpc, timeout_s=180.0)
        walletname = bitcoind.get_prop("walletname")
        _retry_bitcoind_rpc(
            logger, f"createwallet({walletname})", lambda: brpc.proxy.createwallet(walletname)
        )
        seqaddr = _retry_bitcoind_rpc(logger, "getnewaddress", brpc.proxy.getnewaddress)

        # Generate enough blocks to ensure we can fetch the genesis trigger block
        # The default genesis trigger height is 100, so we need at least that many blocks
        min_blocks_needed = settings.genesis_trigger + 10  # Add a buffer

        logger.info(
            f"Generating {min_blocks_needed} blocks for genesis trigger height "
            f"{settings.genesis_trigger}"
        )
        _retry_bitcoind_rpc(
            logger,
            f"generatetoaddress({min_blocks_needed})",
            lambda: brpc.proxy.generatetoaddress(min_blocks_needed, seqaddr),
        )

        # Now generate params with Bitcoin RPC available
        params_gen_data = generate_simple_params(
            initdir, settings, self.n_operators, bitcoind_config
        )
        params = params_gen_data["params"]
        # Instantiaze the generated rollup config so it's convenient to work with.
        rollup_cfg = RollupConfig.model_validate_json(params)

        # Construct the bridge pubkey from the config.
        # Technically, we could use utils::get_bridge_pubkey, but this makes sequencer
        # a dependency of pre-funding logic and just complicates the env setup.
        bridge_pk = get_bridge_pubkey_from_cfg(rollup_cfg)
        # TODO also grab operator keys and launch operators

        if self.pre_generate_blocks > 0:
            if self.pre_fund_addrs:
                # Since the pre-funding is enabled, we have to ensure the amount of pre-generated
                # blocks is enough to deal with the coinbase maturation.
                # Also, leave a log-message to indicate that the setup is little inconsistent.
                if self.pre_generate_blocks < 110:
                    logger.info(
                        "Env setup: pre_fund_addrs is enabled, specify pre_generate_blocks >= 110."
                    )
                    self.pre_generate_blocks = 110

            while self.pre_generate_blocks > 0:
                batch_size = min(self.pre_generate_blocks, 500)

                logger.info(f"Pre generating {batch_size} blocks to address {seqaddr}")
                _retry_bitcoind_rpc(
                    logger,
                    f"generatetoaddress({batch_size})",
                    lambda batch_size=batch_size: brpc.proxy.generatetoaddress(batch_size, seqaddr),
                )
                self.pre_generate_blocks -= batch_size

            if self.pre_fund_addrs:
                # Send funds for btc external and recovery addresses used in the test logic.
                # Generate one more block so the transaction is on the blockchain.
                _retry_bitcoind_rpc(
                    logger,
                    "sendmany(prefund)",
                    lambda: brpc.proxy.sendmany("", {get_address(i): 20 for i in range(20)}),
                )
                _retry_bitcoind_rpc(
                    logger,
                    "generatetoaddress(1)",
                    lambda: brpc.proxy.generatetoaddress(1, seqaddr),
                )

        # generate blocks every 500 millis
        if self.auto_generate_blocks:
            generate_blocks(brpc, BLOCK_GENERATION_INTERVAL_SECS, seqaddr)

        reth_config = RethELConfig(
            rpc_url=f"localhost:{reth_port}",
            secret=reth_secret_path,
        )

        sequencer = seq_fac.create_sequencer_node(bitcoind_config, reth_config, seqaddr, params)

        reth_rpc_http_port = reth.get_prop("eth_rpc_http_port")
        seq_host = sequencer.get_prop("rpc_host")
        seq_port = sequencer.get_prop("rpc_port")
        sequencer_signer = seq_signer_fac.create_sequencer_signer(
            seq_host, seq_port, epoch_gas_limit=self.epoch_gas_limit
        )

        svcs["sequencer"] = sequencer
        svcs["sequencer_signer"] = sequencer_signer
        svcs["reth"] = reth

        # Need to wait for at least `genesis_l1_height` blocks to be generated.
        # Sleeping some more for safety
        if self.auto_generate_blocks:
            time.sleep(BLOCK_GENERATION_INTERVAL_SECS * 10)

        # Wait for sequencer to be ready and ASM to initialize
        logger.info("Waiting for sequencer to be ready...")
        seq_waiter = StrataWaiter(sequencer.create_rpc(), logger, timeout=60, interval=2)
        seq_waiter.wait_until_client_ready()
        logger.info("Waiting for genesis...")
        seq_waiter.wait_until_genesis()
        logger.info("Waiting for ASM worker to initialize and be ready...")
        seq_waiter.wait_until_asm_ready(timeout=60)
        logger.info("ASM worker is operational, environment is ready")

        prover_client_fac = ctx.get_factory("prover_client")
        prover_client_settings = self.prover_client_settings or ProverClientSettings.new_default()
        prover_client = prover_client_fac.create_prover_client(
            bitcoind_config,
            f"http://localhost:{seq_port}",
            f"http://localhost:{reth_rpc_http_port}",
            params,
            prover_client_settings,
        )
        svcs["prover_client"] = prover_client

        return BasicLiveEnv(svcs, bridge_pk, rollup_cfg, params)


class HubNetworkEnvConfig(flexitest.EnvConfig):
    def __init__(
        self,
        pre_generate_blocks: int = 0,
        rollup_settings: RollupParamsSettings | None = None,
        auto_generate_blocks: bool = True,
        n_operators: int = 2,
        duty_timeout_duration: int = 10,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.rollup_settings = rollup_settings
        self.auto_generate_blocks = auto_generate_blocks
        self.n_operators = n_operators
        self.duty_timeout_duration = duty_timeout_duration
        super().__init__()

    def init(self, ctx: flexitest.EnvContext) -> flexitest.LiveEnv:
        btc_fac = ctx.get_factory("bitcoin")
        seq_fac = ctx.get_factory("sequencer")
        seq_signer_fac = ctx.get_factory("sequencer_signer")
        reth_fac = ctx.get_factory("reth")
        fn_fac = ctx.get_factory("fullnode")

        logger = logging.getLogger(__name__)

        bitcoind = btc_fac.create_regtest_bitcoin()
        bitcoind_config = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )
        # wait for services to to startup
        time.sleep(BLOCK_GENERATION_INTERVAL_SECS)

        brpc = _retry_bitcoind_rpc(logger, "create_rpc", bitcoind.create_rpc, timeout_s=180.0)

        walletname = bitcoind.get_prop("walletname")
        _retry_bitcoind_rpc(
            logger, f"createwallet({walletname})", lambda: brpc.proxy.createwallet(walletname)
        )

        seqaddr = _retry_bitcoind_rpc(logger, "getnewaddress", brpc.proxy.getnewaddress)

        if self.pre_generate_blocks > 0:
            logger.info(f"Pre generating {self.pre_generate_blocks} blocks to address {seqaddr}")
            _retry_bitcoind_rpc(
                logger,
                f"generatetoaddress({self.pre_generate_blocks})",
                lambda: brpc.proxy.generatetoaddress(self.pre_generate_blocks, seqaddr),
            )

        # generate blocks every 500 millis
        if self.auto_generate_blocks:
            generate_blocks(brpc, BLOCK_GENERATION_INTERVAL_SECS, seqaddr)

        # set up network params
        initdir = ctx.make_service_dir("_init")
        settings = self.rollup_settings or RollupParamsSettings.new_default().fast_batch()
        params_gen_data = generate_simple_params(
            initdir, settings, self.n_operators, bitcoind_config
        )
        params = params_gen_data["params"]
        # Instantiaze the generated rollup config so it's convenient to work with.
        rollup_cfg = RollupConfig.model_validate_json(params)

        # Construct the bridge pubkey from the config.
        # Technically, we could use utils::get_bridge_pubkey, but this makes sequencer
        # a dependency of pre-funding logic and just complicates the env setup.
        bridge_pk = get_bridge_pubkey_from_cfg(rollup_cfg)
        # TODO also grab operator keys and launch operators

        # reth needs some time to startup, start it first
        secret_dir = ctx.make_service_dir("secret")
        reth_secret_path = os.path.join(secret_dir, "jwt.hex")

        with open(reth_secret_path, "w") as file:
            file.write(generate_jwt_secret())

        reth = reth_fac.create_exec_client(0, reth_secret_path, None)
        seq_reth_rpc_port = reth.get_prop("eth_rpc_http_port")
        fullnode_reth = reth_fac.create_exec_client(
            1, reth_secret_path, f"http://localhost:{seq_reth_rpc_port}"
        )
        reth_authrpc_port = reth.get_prop("rpc_port")

        reth_config = RethELConfig(
            rpc_url=f"localhost:{reth_authrpc_port}",
            secret=reth_secret_path,
        )
        reth_rpc_http_port = reth.get_prop("eth_rpc_http_port")

        sequencer = seq_fac.create_sequencer_node(bitcoind_config, reth_config, seqaddr, params)

        seq_host = sequencer.get_prop("rpc_host")
        seq_port = sequencer.get_prop("rpc_port")
        sequencer_signer = seq_signer_fac.create_sequencer_signer(seq_host, seq_port)

        # Need to wait for at least `genesis_l1_height` blocks to be generated.
        # Sleeping some more for safety
        if self.auto_generate_blocks:
            time.sleep(BLOCK_GENERATION_INTERVAL_SECS * 10)

        fullnode_reth_port = fullnode_reth.get_prop("rpc_port")
        fullnode_reth_config = RethELConfig(
            rpc_url=f"localhost:{fullnode_reth_port}",
            secret=reth_secret_path,
        )

        sequencer_rpc = f"ws://localhost:{sequencer.get_prop('rpc_port')}"

        fullnode = fn_fac.create_fullnode(
            bitcoind_config,
            fullnode_reth_config,
            sequencer_rpc,
            params,
        )

        prover_client_fac = ctx.get_factory("prover_client")
        prover_client_settings = ProverClientSettings.new_with_proving()
        prover_client = prover_client_fac.create_prover_client(
            bitcoind_config,
            f"http://localhost:{seq_port}",
            f"http://localhost:{reth_rpc_http_port}",
            params,
            prover_client_settings,
        )

        svcs = {
            "bitcoin": bitcoind,
            "seq_node": sequencer,
            "sequencer_signer": sequencer_signer,
            "seq_reth": reth,
            "follower_1_node": fullnode,
            "follower_1_reth": fullnode_reth,
            "prover_client": prover_client,
        }

        seq_waiter = StrataWaiter(sequencer.create_rpc(), logger, timeout=60, interval=2)
        seq_waiter.wait_until_client_ready()
        seq_waiter.wait_until_genesis()
        logger.info("Waiting for ASM worker to initialize...")
        seq_waiter.wait_until_asm_ready(timeout=60)
        logger.info("ASM worker is operational")

        fn_waiter = StrataWaiter(fullnode.create_rpc(), logger, timeout=30, interval=2)
        fn_waiter.wait_until_client_ready()
        # TODO: add others like prover, reth, btc

        return BasicLiveEnv(svcs, bridge_pk, rollup_cfg, params)


class DualSequencerMixedPolicyEnvConfig(flexitest.EnvConfig):
    """
    Sets up a test environment with two sequencers (fast vs strict) and a fullnode
    following one of them. Provides flexibility to validate network behavior
    under mixed operational policies.
    """

    def __init__(
        self,
        pre_generate_blocks: int = 0,
        auto_generate_blocks: bool = True,
        n_operators: int = 2,
        fullnode_is_strict_follower: bool = True,
    ):
        self.pre_generate_blocks = pre_generate_blocks
        self.auto_generate_blocks = auto_generate_blocks
        self.n_operators = n_operators
        self.fullnode_is_strict_follower = fullnode_is_strict_follower
        super().__init__()

    def init(self, ctx: flexitest.EnvContext) -> flexitest.LiveEnv:
        self.ctx = ctx

        # 1. Bitcoin regtest setup
        bitcoind, brpc, bitcoind_cfg, addr = self._prepare_bitcoin()

        # 2. Prepare rollup parameters
        init_dir = ctx.make_service_dir("_init")
        params = self._generate_params(init_dir, bitcoind_cfg)
        rollup_cfg_strict = RollupConfig.model_validate_json(params["strict"])
        bridge_pk = get_bridge_pubkey_from_cfg(rollup_cfg_strict)

        # 3. Shared JWT secret for Reth
        secret_dir = ctx.make_service_dir("secret")
        jwt_path = os.path.join(secret_dir, "jwt.hex")
        with open(jwt_path, "w") as f:
            f.write(generate_jwt_secret())

        # 4. Create sequencer bundles
        fast_bundle = self._create_sequencer_bundle(
            name_suffix="fast",
            instance_id=0,
            bitcoind_cfg=bitcoind_cfg,
            jwt_path=jwt_path,
            seqaddr=addr,
            params_json=params["fast"],
        )
        strict_bundle = self._create_sequencer_bundle(
            name_suffix="strict",
            instance_id=1,
            bitcoind_cfg=bitcoind_cfg,
            jwt_path=jwt_path,
            seqaddr=addr,
            params_json=params["strict"],
        )

        # 5. Fullnode creation
        follower = strict_bundle if self.fullnode_is_strict_follower else fast_bundle
        fullnode_bundle = self._create_fullnode_bundle(
            instance_id=0,
            bitcoind_cfg=bitcoind_cfg,
            jwt_path=jwt_path,
            follower_bundle=follower,
            strict_params=params["strict"],
            strict_label="strictfollower",
            fast_label="fastfollower",
        )

        # 6. Aggregate services

        services = {
            "bitcoin": bitcoind,
            "fullnode_reth": fullnode_bundle["fullnode_reth"],
            "fullnode": fullnode_bundle["fullnode"],
            "seq_node_fast": fast_bundle["sequencer"],
            "seq_node_strict": strict_bundle["sequencer"],
            "seq_fast_reth": fast_bundle["reth"],
            "seq_strict_reth": strict_bundle["reth"],
            "sequencer_signer_fast": fast_bundle["sequencer_signer"],
            "sequencer_signer_strict": strict_bundle["sequencer_signer"],
            "prover_client_strict": strict_bundle["prover"],
            "prover_client_fast": fast_bundle["prover"],
        }
        return BasicLiveEnv(services, bridge_pk, rollup_cfg_strict, params["strict"])

    def _generate_params(self, init_dir: str, bitcoind_config: BitcoindConfig) -> dict[str, str]:
        settings_fast = RollupParamsSettings.new_default().fast_batch()
        params_data = generate_simple_params(
            init_dir, settings_fast, self.n_operators, bitcoind_config
        )
        params_fast = params_data["params"]
        params_dict = json.loads(params_fast)
        strict_dict = copy.deepcopy(params_dict)
        strict_dict["proof_publish_mode"] = "strict"
        params_strict = json.dumps(strict_dict)

        return {"fast": params_fast, "strict": params_strict}

    def _prepare_bitcoin(self) -> Any:
        btc_fac = self.ctx.get_factory("bitcoin")
        bitcoind = btc_fac.create_regtest_bitcoin()
        time.sleep(BLOCK_GENERATION_INTERVAL_SECS)
        rpc = bitcoind.create_rpc()
        wallet = "sequencer_wallet"
        rpc.proxy.createwallet(wallet)
        addr = rpc.proxy.getnewaddress()

        if self.pre_generate_blocks > 0:
            rpc.proxy.generatetoaddress(self.pre_generate_blocks, addr)
        if self.auto_generate_blocks:
            generate_blocks(rpc, BLOCK_GENERATION_INTERVAL_SECS, addr)

        cfg = BitcoindConfig(
            rpc_url=f"http://localhost:{bitcoind.get_prop('rpc_port')}/wallet/{wallet}",
            rpc_user=bitcoind.get_prop("rpc_user"),
            rpc_password=bitcoind.get_prop("rpc_password"),
        )
        return bitcoind, rpc, cfg, addr

    def _create_sequencer_bundle(
        self,
        name_suffix: str,
        instance_id: int,
        bitcoind_cfg: BitcoindConfig,
        jwt_path: str,
        seqaddr: str,
        params_json: str,
    ) -> dict[str, Any]:
        ctx = self.ctx
        reth_fac = ctx.get_factory("reth")
        seq_fac = ctx.get_factory("sequencer")
        signer_fac = ctx.get_factory("sequencer_signer")
        prover_fac = ctx.get_factory("prover_client")

        # Exec Reth client
        reth_exec = reth_fac.create_exec_client(
            id=instance_id,
            reth_secret_path=jwt_path,
            sequencer_reth_rpc=None,
            name_suffix=name_suffix,
        )
        eth_port = reth_exec.get_prop("eth_rpc_http_port")
        auth_port = reth_exec.get_prop("rpc_port")
        reth_cfg = RethELConfig(rpc_url=f"localhost:{auth_port}", secret=jwt_path)

        # Sequencer + signer
        sequencer = seq_fac.create_sequencer_node(
            bitcoind_cfg,
            reth_cfg,
            seqaddr,
            params_json,
            multi_instance_enabled=True,
            instance_id=instance_id,
            name_suffix=name_suffix,
        )
        host = sequencer.get_prop("rpc_host")
        port = sequencer.get_prop("rpc_port")
        signer = signer_fac.create_sequencer_signer(
            host,
            port,
            multi_instance_enabled=True,
            instance_id=instance_id,
            name_suffix=name_suffix,
        )

        # Prover client
        prover = prover_fac.create_prover_client(
            bitcoind_cfg,
            f"http://localhost:{port}",
            f"http://localhost:{eth_port}",
            params_json,
            ProverClientSettings.new_with_proving(),
            name_suffix=name_suffix,
        )

        return {
            "reth": reth_exec,
            "reth_cfg": reth_cfg,
            "sequencer": sequencer,
            "sequencer_signer": signer,
            "prover": prover,
            "name_suffix": name_suffix,
        }

    def _create_fullnode_bundle(
        self,
        instance_id: int,
        bitcoind_cfg: BitcoindConfig,
        jwt_path: str,
        follower_bundle: dict[str, Any],
        strict_params: str,
        strict_label: str,
        fast_label: str,
    ) -> dict[str, Any]:
        ctx = self.ctx
        fn_fac = ctx.get_factory("fullnode")
        reth_fac = ctx.get_factory("reth")

        upstream = follower_bundle["reth"]
        upstream_http = f"http://localhost:{upstream.get_prop('rpc_port')}"

        fn_reth = reth_fac.create_exec_client(
            id=instance_id,
            reth_secret_path=jwt_path,
            sequencer_reth_rpc=upstream_http,
            name_suffix="fn",
        )
        fn_reth_cfg = RethELConfig(
            rpc_url=f"localhost:{fn_reth.get_prop('rpc_port')}", secret=jwt_path
        )

        seq_rpc = follower_bundle["sequencer"]
        ws_endpoint = f"ws://localhost:{seq_rpc.get_prop('rpc_port')}"

        label = strict_label if self.fullnode_is_strict_follower else fast_label
        fullnode = fn_fac.create_fullnode(
            bitcoind_cfg,
            fn_reth_cfg,
            ws_endpoint,
            strict_params,
            name_suffix=label,
        )

        return {"fullnode_reth": fn_reth, "fullnode": fullnode}


# TODO: Maybe, we need to make it dynamic to enhance any EnvConfig with load testing capabilities.
class LoadEnvConfig(BasicEnvConfig):
    _load_cfgs: list[LoadConfigBuilder] = []

    def with_load_builder(self, builder: LoadConfigBuilder):
        self._load_cfgs.append(builder)
        return self

    def init(self, ctx: flexitest.EnvContext) -> flexitest.LiveEnv:
        basic_live_env = super().init(ctx)

        if not self._load_cfgs:
            raise Exception(
                "LoadEnv has no load builders! Specify load builders or just use BasicEnv."
            )

        # Create load generator services for all the builders.
        svcs = basic_live_env.svcs
        load_fac = ctx.get_factory("load_generator")
        for builder in self._load_cfgs:
            load_cfg: LoadConfig = builder(svcs)
            svcs[f"load_generator.{builder.name}"] = load_fac.create_simple_loadgen(load_cfg)

        return BasicLiveEnv(svcs, basic_live_env._bridge_pk, basic_live_env._rollup_cfg, basic_live_env._rollup_params_json)

# Functional Tests - New Architecture

Clean, simple functional test suite for Strata.

## Philosophy

**Explicit over implicit. Simple over clever.**

- Tests explicitly start services they need
- No magic setup, no hidden state
- Clear error messages
- Easy to debug

## Quick Start

```bash
# Run all tests
./run_tests.sh

# Run specific test(s)
./run_tests.sh -t test_node_version
./run_tests.sh -t tests/test_node_version.py
./run_tests.sh tests/test_node_version.py
./run_tests.sh -t test_foo test_bar

# Run test group (directory-based)
./run_tests.sh -g bridge
./run_tests.sh -g prover bridge

# List available tests
./run_tests.sh --list

# Keep-alive mode for debugging (starts env and waits, no tests run)
./run_tests.sh --keep-alive basic

# Get help
./run_tests.sh --help
```

## Structure

```
common/       Core library (service, RPC, waiting)
factories/    Service factories
envconfigs/   Environment configs
tests/        Test files
```

## Writing a Test

```python
import flexitest
from common.base_test import StrataNodeTest
from common.config import ServiceType


@flexitest.register
class TestExample(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")  # Use basic environment

    def run(self) -> bool:
        # Access to runcontext is available via self.runctx
        # (set automatically by BaseTest.main)

        # Get services using ServiceType enum
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        # Create RPC clients
        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata.create_rpc()

        # Wait for ready
        self.wait_for_rpc_ready(strata_rpc)

        # Do test logic
        version = strata_rpc.strata_protocolVersion()
        assert version == 1

        return True
```

## Core Utilities

### Waiting

```python
# Simple condition wait
self.wait_for(lambda: service.is_ready(), timeout=30)

# Wait for RPC
self.wait_for_rpc_ready(rpc, method="strata_protocolVersion")

# Custom wait
from common.wait import wait_until

wait_until(condition, error_with="Custom error", timeout=30, step=0.5)
```

### RPC Calls

```python
# Attribute style (uses __getattr__ for dynamic method dispatch)
version = rpc.strata_protocolVersion()
balance = rpc.eth_getBalance("0x123...", "latest")

# Explicit style
version = rpc.call("strata_protocolVersion")
balance = rpc.call("eth_getBalance", "0x123...", "latest")
```

### Service Access

```python
from common.config import ServiceType

# Get service from test (uses self.runctx internally)
bitcoin = self.get_service(ServiceType.Bitcoin)

# Access properties
rpc_port = bitcoin.get_prop("rpc_port")
datadir = bitcoin.get_prop("datadir")

# Create RPC client
rpc = bitcoin.create_rpc()
```

## Environment Configs

Environments define which services to start:

```python
# envconfigs/basic.py
from common.config import ServiceType


class BasicEnvConfig(flexitest.EnvConfig):
    def init(self, ectx: flexitest.EnvContext):
        btc_factory = ectx.get_factory(ServiceType.Bitcoin)
        strata_factory = ectx.get_factory(ServiceType.Strata)

        # Start Bitcoin
        bitcoin = btc_factory.create_regtest()
        bitcoin.wait_for_ready(timeout=10)

        # Start Strata
        strata = strata_factory.create_node(...)
        strata.wait_for_ready(timeout=10)

        return flexitest.LiveEnv(
            {
                ServiceType.Bitcoin: bitcoin,
                ServiceType.Strata: strata,
            }
        )
```

Use in tests:

```python
def __init__(self, ctx: flexitest.InitContext):
    ctx.set_env("basic")
```

## Factories

Factories create services. They should be dumb - just build command and start process.

```python
# factories/bitcoin.py
from common.services import BitcoinServiceWrapper, BitcoinServiceProps


class BitcoinFactory(flexitest.Factory):
    @flexitest.with_ectx("ctx")
    def create_regtest(self, **kwargs):
        ctx: flexitest.EnvContext = kwargs["ctx"]

        # Set up service directories and ports
        datadir = ctx.make_service_dir(ServiceType.Bitcoin)
        rpc_port = self.next_port()
        logfile = os.path.join(datadir, "service.log")

        # Build command
        cmd = ["bitcoind", "-regtest", f"-rpcport={rpc_port}", ...]

        # Create props (validated by dataclass)
        props = BitcoinServiceProps(
            rpc_port=rpc_port,
            rpc_url=f"http://localhost:{rpc_port}",
            datadir=datadir,
        )

        # Create service wrapper with RPC factory
        svc = BitcoinServiceWrapper(
            props, cmd, stdout=logfile, rpc_factory=lambda: BitcoindClient(...)
        )
        svc.start()
        return svc
```

## Running Tests

### Test Selection

Tests can be filtered by name or group (directory structure):

```bash
# Run all tests
./run_tests.sh

# Run specific tests by name (basename; paths or positional are OK)
./run_tests.sh -t test_node_version
./run_tests.sh -t tests/test_node_version.py
./run_tests.sh tests/test_node_version.py
./run_tests.sh -t test_foo test_bar test_baz

# Run tests by group (subdirectory under tests/)
# Example: tests/bridge/test_deposit.py is in group "bridge"
./run_tests.sh -g bridge
./run_tests.sh -g prover bridge sync

# Combine filters (tests OR groups)
./run_tests.sh -t test_node_version -g bridge

# List all available tests and groups
./run_tests.sh --list
```

### Keep-Alive Mode

For debugging, start an environment and keep it running:

```bash
./run_tests.sh --keep-alive basic
```

This starts all services in the "basic" environment and keeps them alive until you press Ctrl+C. **No tests are run** - this is purely for debugging. Useful for:
- Manual testing via RPC
- Inspecting service state
- Debugging service startup issues

Service connection info will be printed on startup.

### Disabling Tests

Permanently disabled tests are defined in `entry.py` `disabled_tests()`. For temporary disabling (e.g., local debugging), use the `DISABLED_TESTS` environment variable:

```bash
# Disable single test
DISABLED_TESTS=test_flaky ./run_tests.sh

# Disable multiple tests (comma-separated)
DISABLED_TESTS=test_foo,test_bar,test_flaky ./run_tests.sh

# Works with other flags
DISABLED_TESTS=test_slow ./run_tests.sh -t test_fast test_slow
```

The env var extends the base disabled list, so both are applied.

## Debugging

### Service Logs

Logs are in test data directory:

```
_dd/
  <test_run_id>/        # Unique ID for each test run
    <env_name>/         # Environment name (e.g., "basic")
      bitcoin/service.log
      <strata_service>/service.log  # e.g., strata_sequencer
```

Example: `_dd/9-13-wpbec/basic/bitcoin/service.log`

### Test Logs

Each test gets its own logger:

```python
self.info("Something happened")
self.debug("Debug info")
self.error("Error occurred")
```

Set log level with environment variable:
```bash
LOG_LEVEL=DEBUG ./run_tests.sh
```

### Common Issues

**RPC not ready**: Increase timeout or check service logs
```python
self.wait_for_rpc_ready(rpc, timeout=60)
```

**Service crashed**: Check `service.log` in datadir

**Timeout errors**: Check exception message for last error and attempt count

## Repeatable OL upgrade test

`test_checkpoint_predicate_rotation` creates a fresh V1 network on every run and
rotates it to V2. Ordinary functional CI discovers and runs it with native
checkpoint programs. The programs execute the fixed-spec checkpoint logic and
sign with different BIP-340 test keys, so stale-key routing cannot pass verification.
This checks integration and recovery without generating a ZK proof.

```sh
./run_tests.sh -t test_checkpoint_predicate_rotation
```

The test keeps an old checkpoint in flight through enactment by mining empty
Bitcoin blocks, checks that the handover epoch ends exactly at the enactment
height, kills and restarts the sequencer before settlement, then requires the
handover and two V2 epochs to finalize. A live L1 follower and a fresh follower
started after the upgrade replay the same checkpoints and account summaries and
read V2 from their reconstructed root state. Non-sequencer nodes currently use
checkpoint sync; block-executing fullnode replay is not available yet.

Real SP1 network proving is opt-in. Install the SP1 toolchain matching the
workspace SDK (`v6.8.1`) and export `NETWORK_PRIVATE_KEY` through your shell's
secret configuration. On macOS, use LLVM's archiver for the guest's RISC-V
objects; for a Homebrew LLVM installation:

```sh
export AR_riscv64im_unknown_none_elf="$(brew --prefix llvm)/bin/llvm-ar"
```

Then, from `functional-tests/`, run:

```sh
OL_PROVER_BACKEND=sp1 SP1_PROVER=network \
./run_tests.sh -t test_checkpoint_predicate_rotation
```

The runner builds the node and tools in release mode with SP1 support, then
builds the current V2 checkpoint guest, predicate, and artifact manifest. V1
comes from the [frozen deployed bundle](testdata/sp1/v1/README.md). Both the
V2 build and the test network use the runtime parameters stored beside V1;
the test's genesis predicate and rotation target come from their respective
bundles. The node validates both bundles against those parameters and derives
their keys from the ELFs at startup. A custom `CARGO_TARGET_DIR` is supported.

The node inherits `NETWORK_PRIVATE_KEY`, optional `NETWORK_RPC_URL`, and
`SP1_PROOF_STRATEGY` from the invoking shell or Actions step. The network key
signs proof requests; it is separate from the local test's sequencer and
Bitcoin keys. The runner rejects missing credentials instead of falling back
to native proving. Real proving submits paid network requests.

SP1 mode uses slower blocks and longer proof waits to accommodate the network.

### Running the SP1 upgrade test in CI

The dedicated daily/manual SP1 workflow is added in the next review chunk.
The existing ordinary functional CI job still runs this test in native mode;
triggering that job does not enable network proving.

Configure a repository Actions secret named `NETWORK_PRIVATE_KEY`, or give
this repository access to an organization Actions secret with that name.
GitHub must pass the secret into the test step explicitly. No local network
key is needed to trigger CI.

The CI runner needs the usual functional-test dependencies (Rust, bitcoind,
uv, and protoc), plus the matching SP1 `v6.8.1` toolchain. With those installed,
the SP1 job runs only the upgrade test using this step:

```yaml
- name: Run V1 to V2 upgrade with SP1 network proofs
  working-directory: functional-tests
  env:
    OL_PROVER_BACKEND: sp1
    SP1_PROVER: network
    NETWORK_PRIVATE_KEY: ${{ secrets.NETWORK_PRIVATE_KEY }}
  run: ./run_tests.sh -t test_checkpoint_predicate_rotation
```

The script builds the release binaries and V2 bundle before running the test;
V1 comes from committed test data. A missing secret fails before the builds
start. If the network needs a custom endpoint or fulfillment strategy, pass
`NETWORK_RPC_URL` or `SP1_PROOF_STRATEGY` into the same step's environment.

Once the dedicated workflow is available, use its **Run workflow** action for
the first real-proof validation. The daily schedule runs after the workflow
lands on the default branch. Check the test result and service logs to verify
that the V1 handover and both V2 checkpoints finalized; a successful guest
build alone does not validate network proving.

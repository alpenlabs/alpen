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

        return flexitest.LiveEnv({
            ServiceType.Bitcoin: bitcoin,
            ServiceType.Strata: strata,
        })
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
            props, cmd, stdout=logfile,
            rpc_factory=lambda: BitcoindClient(...)
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

# Run specific tests by name (without .py extension)
./run_tests.sh -t test_node_version
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

To temporarily disable flaky or WIP tests, add them to `disabled_tests()` in `entry.py`:

```python
def disabled_tests() -> frozenset[str]:
    return frozenset(["test_flaky", "test_wip"])
```

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

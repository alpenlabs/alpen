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
# Install dependencies
pip install -r requirements.txt

# Run all tests
./run_tests.sh

# Run specific test
python entry.py -t test_node_basic
```

## Structure

```
lib/          Core library (service, RPC, waiting)
factories/    Service factories
env/          Environment configs
tests/        Test files
```

## Writing a Test

```python
import flexitest
from tests.base import BaseTest

@flexitest.register
class TestExample(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")  # Use basic environment

    def main(self, ctx: flexitest.RunContext):
        # Get services
        bitcoin = ctx.get_service("bitcoin")
        strata = ctx.get_service("strata")

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
wait_until(condition, timeout=30, error_msg="Custom error")
```

### RPC Calls

```python
# Attribute style
version = rpc.strata_protocolVersion()
balance = rpc.eth_getBalance("0x123...", "latest")

# Explicit style
version = rpc.call("strata_protocolVersion")
```

### Service Access

```python
# Get service from context
bitcoin = ctx.get_service("bitcoin")

# Access properties
rpc_port = bitcoin.get_prop("rpc_port")
datadir = bitcoin.get_prop("datadir")

# Create RPC client
rpc = bitcoin.create_rpc()
```

## Environment Configs

Environments define which services to start:

```python
# env/configs.py
class BasicEnv(flexitest.EnvConfig):
    def init(self, ctx):
        bitcoin = bitcoin_factory.create_regtest()
        strata = strata_factory.create_node(...)

        return flexitest.LiveEnv({
            "bitcoin": bitcoin,
            "strata": strata,
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
class BitcoinFactory(flexitest.Factory):
    @flexitest.with_ectx("ctx")
    def create_regtest(self, ctx):
        # Build command
        cmd = ["bitcoind", "-regtest", ...]

        # Create service using flexitest's ProcService
        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.start()
        return svc
```

## Debugging

### Service Logs

Logs are in test data directory:

```
_test_data/
  <test_name>/
    bitcoin/service.log
    strata/service.log
```

### Test Logs

Each test gets its own logger:

```python
self.info("Something happened")
self.debug("Debug info")
self.error("Error occurred")
```

### Common Issues

**RPC not ready**: Increase timeout or check service logs
```python
self.wait_for_rpc_ready(rpc, timeout=60)
```

**Service crashed**: Check `service.log` in datadir

**Timeout errors**: Check `error_msg` in exception for last error

## TODO

- [ ] Complete StrataFactory (waiting for binary interface)
- [ ] Add Reth factory if needed
- [ ] Create more environment configs
- [ ] Add bridge test helpers
- [ ] Add more test utilities as needed

## See Also

- `PLAN.md` - Detailed architecture plan
- Old `functional-tests/` - Previous implementation (for reference only)

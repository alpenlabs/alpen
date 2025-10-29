# Strata Functional Tests

Tests will be added here when we have more functionality to test.

## Prerequisites

### `bitcoind`

Most tests depend upon `bitcoind` being available. The tests here execute
this binary and then, perform various tests.

```bash
# for macOS
brew install bitcoin
```

Note that in macOS, you may need to specifically add a firewall rule to allow incoming local `bitcoind` connections.

```bash
# for Linux (x86_64)
curl -fsSLO --proto "=https" --tlsv1.2 https://bitcoincore.org/bin/bitcoin-core-29.0/bitcoin-29.0-x86_64-linux-gnu.tar.gz
tar xzf bitcoin-29.0-x86_64-linux-gnu.tar.gz
sudo install -m 0755 -t /usr/local/bin bitcoin-29.0/bin/*
# remove the files, as we just copied it to /bin
rm -rf bitcoin-29.0 bitcoin-29.0-x86_64-linux-gnu.tar.gz
```

```bash
# check installed version
bitcoind --version
```

### `uv`

> [!NOTE]
> Make sure you have installed Python 3.10 or higher.

We use [`uv`](https://github.com/astral-sh/uv) for managing the test dependencies.

First, install `uv` following the instructions at <https://docs.astral.sh/uv/>.


Check, that `uv` is installed:

```bash
uv --version
```

Now you can run tests with:

```bash
uv run python entry.py
````


### Rosetta

On macOS, you must have Rosetta emulation installed in order to compile the `solx` dependency:

```bash
# macOS only
softwareupdate --install-rosetta
```

## Running tests

You can run all tests:

```bash
./run_test.sh
```

You also can run a specific test:

```bash
./run_test.sh -t tests/bridge/bridge_deposit_happy.py
```

Or (shorter version),

```bash
./run_test.sh -t bridge/bridge_deposit_happy.py
```

Or, you can run a specific test group:

```bash
./run_test.sh -g bridge
```

The full list of arguments for running tests can be viewed by:

```bash
./run_test.sh -h
```

## Running prover tasks

```bash
PROVER_TEST=1 ./run_test.sh -g prover
```

The test harness script will be extended with more functionality as we need it.

## Running with code coverage

```bash
CI_COVERAGE=1 ./run_test.sh
```

Code coverage artifacts (`*.profraw` files) are generated in `target/llvm-cov-target/`.
Binaries and other build artifacts are generated in `target/llvm-cov-target/debug`.


## Keep-alive env setup

During development it's quite handy to have local services spin up quickly,
instead of bothering with Docker's (build time is heavy if built from scratch).

To do that, you can use the following command:
```bash
./run_test.sh -e <env_name>
```

For instance:
```bash
./run_test.sh -e basic
```

As a result, services will be kept alive, so you can send RPCs and play around.

## Test Environment Configurations

The functional tests support multiple environment configurations, each designed for specific testing scenarios:

### **"basic"**
- **Purpose**: Default environment for most tests
- **Components**:
  - Bitcoin regtest node
  - Sequencer + Sequencer Signer
  - Reth (Ethereum execution client)
  - Prover Client
- **Settings**: 110 pre-generated blocks, fast batch mode
- **Use case**: Standard rollup functionality tests

### **"load_reth"**
- **Purpose**: Load testing with state diff generation
- **Components**: Same as basic + Load Generator service
- **Settings**: State diff generation enabled, load jobs (30/sec rate)
- **Use case**: Performance testing, benchmarking

### **"hub1"**
- **Purpose**: Multi-node network testing
- **Components**:
  - Bitcoin regtest node
  - **Sequencer node** + Sequencer Signer + Reth
  - **Full node follower** + separate Reth instance
  - Prover Client
- **Use case**: Testing network synchronization, follower behavior

### **"prover"**
- **Purpose**: Testing with strict proof validation
- **Components**: Same as basic
- **Settings**: **Strict mode** (no proof timeout), proving enabled
- **Use case**: Zero-knowledge proof validation tests

### **Other environments**:
- **"operator_lag"**: Tests operator delays (10min message interval)
- **"devnet"**: Production devnet configuration
- **"crash"**: For crash/recovery testing
- **"state_diffs"**: State diff generation testing

Each environment spins up the appropriate services and configures them for specific testing scenarios, from basic functionality to complex multi-node networks and performance testing.

For more details on environment configurations, see [entry.py](entry.py).


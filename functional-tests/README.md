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
curl -fsSLO --proto "=https" --tlsv1.2 https://bitcoincore.org/bin/bitcoin-core-28.0/bitcoin-28.0-x86_64-linux-gnu.tar.gz
tar xzf bitcoin-28.0-x86_64-linux-gnu.tar.gz
sudo install -m 0755 -t /usr/local/bin bitcoin-28.0/bin/*
# remove the files, as we just copied it to /bin
rm -rf bitcoin-28.0 bitcoin-28.0-x86_64-linux-gnu.tar.gz
```

```bash
# check installed version
bitcoind --version
```

### Poetry

> [!NOTE]
> Make sure you have installed Python 3.10 or higher.

We use Poetry for managing the test dependencies.

First, install `poetry`:

```bash
# install via apt
apt install python3-poetry
# or install poetry via pip3
pip3 install poetry
# or install poetry via pipx
pipx install poetry
# or install poetry via homebrew
brew install poetry
```

Check, that `poetry` is installed:

```bash
poetry --version
```

Finally, install all test dependencies (without installing the root package):

```bash
poetry install --no-root
```

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

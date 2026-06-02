# Strata Datatool

This is a tool for doing basic operations with Strata keys and data.

## Usage

The basic flow to generate the params files looks like this:

```sh
# Generate keys for the different parties each on different machines.
strata-datatool genxpriv sequencer.bin
strata-datatool genxpriv operator1.bin
strata-datatool genxpriv operator2.bin

# Generate the pubkeys, also on their original machines.
strata-datatool genseqpubkey -f sequencer.bin
strata-datatool genoppubkey -f operator1.bin
strata-datatool genoppubkey -f operator2.bin

# Generate a genesis L1 view file (requires Bitcoin RPC connection).
cargo build --bin strata-datatool --features btc-client
strata-datatool \
    --bitcoin-rpc-url http://localhost:18332 \
    --bitcoin-rpc-user rpcuser \
    --bitcoin-rpc-password rpcpass \
    genl1view \
    --genesis-l1-height 100 \
    --output genesis_l1_view.json

# Generate the OL params (provides the genesis OL block id consumed by ASM params).
strata-datatool gen-ol-params \
    --genesis-l1-view-file genesis_l1_view.json \
    -o ol-params.json

# Generate the ASM params from the operator/sequencer pubkeys and the OL params.
strata-datatool gen-asm-params \
    -n 'hello-world-network' \
    -s <sequencer-x-only-pubkey> \
    -b <operator1-compressed-pubkey> \
    -b <operator2-compressed-pubkey> \
    --ol-params ol-params.json \
    --safe-harbour-address <p2tr-bosd-descriptor> \
    --genesis-l1-view-file genesis_l1_view.json \
    -o asm-params.json
```

## Envvars

Alternatively, instead of passing `-f`, you can pass `-E` and define either
`STRATA_SEQ_KEY` or `STRATA_OP_KEY` to pass the seed keys to the program.

## Generating VerifyingKey

Before proceeding, make sure that you have SP1 correctly set up by following the installation instructions provided [here](https://docs.succinct.xyz/docs/sp1/getting-started/install)

The checkpoint verifying key is baked into the ASM checkpoint predicate. To ensure it is the correct verifying key, build the binary in release mode and confirm that SP1 is set up correctly by following its installation instructions.

For production usage—since SP1 verification key generation is platform and workspace dependent—build the data tool in release mode with the sp1-docker-builder feature:

```bash
cargo build --bin strata-datatool -F "sp1-docker-builder" --release
```

Because building the guest code in Docker can be time-consuming, you can generate the verification key locally for testing or development using:

```bash
cargo build --bin strata-datatool -F "sp1-builder" --release
```

Print the resolved checkpoint predicate (the SP1 verifying key under `sp1-builder`):

```bash
strata-datatool gen-checkpoint-predicate
```

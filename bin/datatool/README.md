# Strata Datatool

This is a tool for doing basic operations with Strata keys and data.

## Usage

The basic flow to generate a params file with it looks like this:

```sh
# Generate keys for the different parties each on different machines.
strata-datatool genxpriv sequencer.bin
strata-datatool genxpriv operator1.bin
strata-datatool genxpriv operator2.bin

# Generate the pubkeys, also on their original machines.
strata-datatool genseqpubkey -f sequencer.bin
strata-datatool genopxpub -f operator1.bin
strata-datatool genopxpub -f operator2.bin

# Take the generated pubkeys and generate the params file with it.
# Option 1: With Bitcoin RPC connection (fetches genesis L1 view from Bitcoin node)
cargo build --bin strata-datatool --features btc-client
strata-datatool \
    --bitcoin-rpc-url http://localhost:18332 \
    --bitcoin-rpc-user rpcuser \
    --bitcoin-rpc-password rpcpass \
    genparams \
    -n 'hello-world-network' \
    -s XGUgTAJNpexzrjgnbMvGtDBCZEwxd6KQE4PNDWE6YLZYBTGoS \
    -b tpubDASVk1m5cxpmUbwVEZEQb8maDVx9kDxBhSLCqsKHJJmZ8htSegpHx7G3RFudZCdDLtNKTosQiBLbbFsVA45MemurWenzn16Y1ft7NkQekcD \
    -b tpubDBX9KQsqK2LMCszkDHvANftHzhJdhipe9bi9MNUD3S2bsY1ikWEZxE53VBgYN8WoNXk9g9eRzhx6UfJcQr3XqkA27aSxXvKu5TYFZJEAjCd \
    --genesis-l1-height 100 \
    -o params.json

# Option 2: Using pre-generated genesis L1 view file (no Bitcoin RPC needed)
strata-datatool genparams \
    -n 'hello-world-network' \
    -s XGUgTAJNpexzrjgnbMvGtDBCZEwxd6KQE4PNDWE6YLZYBTGoS \
    -b tpubDASVk1m5cxpmUbwVEZEQb8maDVx9kDxBhSLCqsKHJJmZ8htSegpHx7G3RFudZCdDLtNKTosQiBLbbFsVA45MemurWenzn16Y1ft7NkQekcD \
    -b tpubDBX9KQsqK2LMCszkDHvANftHzhJdhipe9bi9MNUD3S2bsY1ikWEZxE53VBgYN8WoNXk9g9eRzhx6UfJcQr3XqkA27aSxXvKu5TYFZJEAjCd \
    --genesis-l1-view-file genesis_l1_view.json \
    -o params.json

# Generate a genesis L1 view file (requires Bitcoin RPC connection)
cargo build --bin strata-datatool --features btc-client
strata-datatool \
    --bitcoin-rpc-url http://localhost:18332 \
    --bitcoin-rpc-user rpcuser \
    --bitcoin-rpc-password rpcpass \
    genl1view \
    --genesis-l1-height 100 \
    --output genesis_l1_view.json
```

## Envvars

Alternatively, instead of passing `-f`, you can pass `-E` and define either
`STRATA_SEQ_KEY` or `STRATA_OP_KEY` to pass the seed keys to the program.

## Generating VerifyingKey

Before proceeding, make sure that you have SP1 correctly set up by following the installation instructions provided [here](https://docs.succinct.xyz/docs/sp1/getting-started/install)

To ensure that the RollupParams contain the correct verifying key, build the binary in release mode and confirm that SP1 is set up correctly by following its installation instructions.

For production usage—since SP1 verification key generation is platform and workspace dependent—build the data tool in release mode with the sp1-docker-builder feature:

```bash
cargo build --bin strata-datatool -F "sp1-docker-builder" --release
```

Because building the guest code in Docker can be time-consuming, you can generate the verification key locally for testing or development using:

```bash
cargo build --bin strata-datatool -F "sp1-builder" --release
```

Additionally, the generated ELF can be exported after building the datatool as specified above:

```bash
strata-datatool genparams --elf-dir <ELF-PATH>
```

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

# Generate the genesis L1 anchor file (requires Bitcoin RPC connection).
cargo build --bin strata-datatool --features btc-client
strata-datatool \
    --bitcoin-rpc-url http://localhost:18332 \
    --bitcoin-rpc-user rpcuser \
    --bitcoin-rpc-password rpcpass \
    gen-l1-anchor \
    --genesis-l1-height 100 \
    --output l1-anchor.json

# Generate the Alpen params artifact consumed by alpen-client and referenced by
# OL params generation.
strata-datatool gen-alpen-params \
    --alpen-chain-config alpen-chain.json \
    --bridge-denomination-sats 200000000 \
    --max-withdrawal-descriptor-len 81 \
    -o alpen-params.json

# Generate the OL params (provides the genesis OL block id consumed by ASM params).
strata-datatool gen-ol-params \
    --l1-anchor-file l1-anchor.json \
    --alpen-params alpen-params.json \
    -o ol-params.json

# Generate the ASM params from the operator/sequencer pubkeys and the OL params.
strata-datatool gen-asm-params \
    -n 'hello-world-network' \
    -s <sequencer-x-only-pubkey> \
    -b <operator1-compressed-pubkey> \
    -b <operator2-compressed-pubkey> \
    --ol-params ol-params.json \
    --safe-harbour-address <p2tr-bosd-descriptor> \
    --l1-anchor-file l1-anchor.json \
    -o asm-params.json \
    --cli-config alpen-cli-profile.toml
```

## Alpen CLI network profile

`gen-asm-params --cli-config <path>` additionally emits the network fields the
`alpen` wallet CLI reads from its `config.toml`, derived from the same inputs
as the ASM and OL params so they cannot drift apart:

- `network` — the L1 network from the genesis anchor
- `magic_bytes` — the SPS-50 magic bytes
- `bridge_pubkey` — the aggregated MuSig2 key of the bridge operator set
- `bridge_denomination_sats` — the bridge denomination, used for both deposits
  and withdrawals
- `recovery_delay` — the deposit-request reclaim delay in Bitcoin blocks
- `max_withdrawal_amount_sats` and `max_withdrawal_descriptor_len` — the OL's
  withdrawal limits, from `--ol-params`

Deposits are locked at the ASM bridge denomination while withdrawals are
validated by the OL STF against the OL bridge denomination, so the two are one
network value: `--deposit-sats` defaults to the `--ol-params` denomination and
`gen-asm-params` rejects a value that differs from it. Withdrawals are batched
in multiples of that denomination, capped at `max_withdrawal_amount_sats` when
the OL params set a cap. For uncapped networks, the generated snippet omits
`max_withdrawal_amount_sats`.

Merge the generated snippet into the CLI's `config.toml`, replacing any of
these keys the file already defines — TOML rejects duplicate keys, so
appending the snippet to a config that already has `bridge_pubkey` fails to
parse. The command refuses to overwrite an existing file, so don't point it
at a live config.

These values are consensus-critical: hand-editing them can produce deposit
transactions the bridge won't recognize, or withdrawals the OL rejects.

### Migrating CLI configs from the ASM params file

The wallet CLI previously read these values from an `asm-params.json` file
resolved via the `STRATA_NETWORK_PARAMS` environment variable or the
`asm_params_path` config key; both are removed. Existing deployments must add
the fields above to `config.toml`. The former
`withdrawal_denomination_sats` key is replaced by `bridge_denomination_sats`,
which now drives both deposits and withdrawals.

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

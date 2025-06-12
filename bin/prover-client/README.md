# Strata Prover Client

The `strata-prover-client` handles fetching the witness from the EE and Sequencer, generating proofs, and storing the generated proofs.

## Building the Project

To build the project, run the following command:

```bash
cargo run --bin strata-prover-client -F "prover" -- [OPTIONS]
```

Available Options

- `--rpc-port`: The JSON-RPC port to use (optional, default: 4844)
- `--sequencer-rpc`: The RPC host and port for the sequencer (required)
- `--reth-rpc`: The RPC host and port for the Reth node (required)
- `--enable-dev-rpcs`: Enable or disable the prover client dev RPCs (default: true)

## Example Usage

```bash
cargo run --bin strata-prover-client -F "prover" -- \
  --rpc-port 8545 \
  --sequencer-rpc http://sequencer.local:8545 \
  --reth-rpc http://reth.local:8545
```

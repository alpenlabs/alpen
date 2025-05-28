# Building the Prover Client Docker Images

## Native Mode

Generate a Prover Image where proving occurs in native mode. In native mode, instead of actual proof generation, proof statements are executed in native Rust. This results in an empty proof and the expected public parameters.

```bash
docker build  --build-arg PROVER_FEATURES=default -t prover-client-native -f ./docker/prover-client/Dockerfile .
```

## `sp1-mock` Mode

Generate a Prover Image in `sp1-mock` proving mode.
In mock mode, execution happens inside the RISC-V VM, generating a mock proof and the expected public parameters.

```bash
docker build  --build-arg PROVER_FEATURES=sp1-mock -t prover-client-sp1-mock -f ./docker/prover-client/Dockerfile .
```

## `sp1` Mode

Generate a Prover Image in `sp1` proving mode.
In `sp1` mode, actual proofs and public parameters are generated.

```bash
docker build  --build-arg PROVER_FEATURES=sp1 -t prover-client-sp1 -f ./docker/prover-client/Dockerfile .
```

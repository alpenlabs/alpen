# Contributor docs

These pages explain how the OL node is put together, and link to the code for the details.

- [Architecture](architecture.md): the state machines and accounts, the node's services, the
  checkpoint lifecycle, storage and configuration.
- [Repository layout](layout.md): what each crate is for.
- [OL specs](ol-specs.md): spec dispatch and selection, the state root and adding a spec.

Tool-specific guides live next to the code:

- [Functional tests](../functional-tests/README.md)
- [SP1 guest builder](../provers/sp1/README.md)
- [Checkpoint artifacts](../crates/ol/checkpoint-artifacts/README.md)
- [datatool](../bin/datatool/README.md)
- [dbtool](../bin/strata-dbtool/README.md)
- [Prover perf](../bin/prover-perf/README.md)
- [Docker](../docker/README.md)

If you change behavior a page describes, update the page in the same PR.

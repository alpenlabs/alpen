# Minimal runtime image wrapping the strata-datatool binary.
#
# The binary is pre-built on the CI runner with `--features sp1-builder`
# (see .github/workflows/ci-build.yml) and staged into
# docker/strata/artifacts/ alongside the guest ELFs. This image is meant
# to be consumed by downstream pipelines (deployments/generate-params.yml,
# bridge, opnet, etc.) that need to invoke datatool without bundling an
# SP1 toolchain or rebuilding from source.
#
# Example:
#   docker run --rm \
#     -v $(pwd):/out \
#     strata-datatool:<sha> \
#     -b signet \
#     --bitcoin-rpc-url http://... --bitcoin-rpc-user ... --bitcoin-rpc-password ... \
#     genparams --checkpoint-predicate sp1-groth16 \
#       --elf-dir /out/elfs --genesis-l1-view-file /out/l1view.json \
#       > /out/rollup-params.json

FROM debian:13-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 curl jq && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /app --uid 10001 --shell /usr/sbin/nologin alpen

WORKDIR /app

COPY docker/strata/artifacts/strata-datatool /usr/local/bin/strata-datatool
RUN chmod 0555 /usr/local/bin/strata-datatool

USER alpen

ENTRYPOINT ["/usr/local/bin/strata-datatool"]

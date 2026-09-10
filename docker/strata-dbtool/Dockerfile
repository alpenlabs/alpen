# Maintenance image wrapping the strata-dbtool binary.
#
# strata-dbtool inspects and repairs the OL and EE datastores directly. It is
# NOT a service: it is run ad hoc against a node's PVCs, normally with that
# node scaled to zero, e.g.
#
#   kubectl scale sts alpen-client-ee -n rollup --replicas=0
#   kubectl apply -f strata-dbtool-pod.yaml     # mounts ee-data + ee-sled
#   kubectl exec -it strata-dbtool -n rollup -- strata-dbtool <subcommand>
#
# Build pattern mirrors docker/strata-signer/Dockerfile (cargo-chef, then a
# debian:13-slim runtime).
#
# NO `USER alpen` HERE, DELIBERATELY — unlike every other image in this repo.
# dbtool operates on datadirs written by nodes that run as root
# (the EE sequencer's pod securityContext is runAsUser: 0), so a uid-10001
# runtime user cannot read, let alone repair, those files. The pod spec is
# expected to set runAsUser: 0 as well; leaving the image root avoids a
# silent-permission-denied class of failure during an incident.
#
# ENTRYPOINT is the binary, so `docker run <img> --help` works, but debug pods
# typically override command with ["sleep","infinity"] and exec in.

ARG RUST_VERSION=1.92

# base
FROM rust:${RUST_VERSION}-slim-trixie AS chef
WORKDIR /app

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang libclang-dev pkg-config libssl-dev protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

# Install cargo-chef once (cached layer)
RUN cargo install cargo-chef --locked

# ---- planner: generate recipe.json ----
FROM chef AS planner
COPY rust-toolchain.toml ./
RUN rustup toolchain install
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- builder ----
FROM chef AS builder
COPY rust-toolchain.toml ./
RUN rustup toolchain install

# Bring in the recipe from planner
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo build --locked --release --bin strata-dbtool && \
    install -m 0555 /app/target/release/strata-dbtool /usr/local/bin/strata-dbtool

# ---- runtime ----
FROM debian:13-slim AS runtime

ENV APP_HOME=/app \
    HOME=/app \
    RUST_LOG=info

# jq and less are here on purpose: dbtool emits JSON and this image exists to be
# poked at interactively during an incident.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 curl jq less && \
    rm -rf /var/lib/apt/lists/*

WORKDIR "${APP_HOME}"

COPY --from=builder /usr/local/bin/strata-dbtool /usr/local/bin/strata-dbtool
RUN chmod 0555 /usr/local/bin/strata-dbtool

ENTRYPOINT ["/usr/local/bin/strata-dbtool"]

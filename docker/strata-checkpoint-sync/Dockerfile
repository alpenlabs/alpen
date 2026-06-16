################################################################################
# Checkpoint-sync Strata image.
#
# Builds the `strata` binary without default features. A checkpoint-sync node
# reconstructs OL state from L1-buried checkpoints; it never assembles blocks,
# runs the writer/broadcaster, or proves checkpoints, so that code is left out
# of the binary entirely.
#
# This image bundles neither strata-datatool nor the SP1 guest ELFs.
#
# Run with a config where `is_sequencer = false`. The entrypoint rejects a
# config with `is_sequencer = true` because this binary has no sequencer code.
################################################################################

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

# Install cargo-chef (cached layer)
RUN cargo install cargo-chef --locked

# ---- planner: generate recipe.json ----
FROM chef AS planner
COPY rust-toolchain.toml ./
RUN rustup toolchain install
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- builder
FROM chef AS builder
COPY rust-toolchain.toml ./
RUN rustup toolchain install

# Bring in the recipe from planner
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo chef cook --release --no-default-features --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo build --locked --release --bin strata --no-default-features && \
    install -m 0555 /app/target/release/strata /usr/local/bin/strata


# ---- runtime ----
FROM debian:13-slim AS runtime

ENV APP_HOME=/app \
    HOME=/app \
    RUST_LOG=info

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 curl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir "${APP_HOME}" --uid 10001 --shell /usr/sbin/nologin alpen

WORKDIR "${APP_HOME}"

# Ensure writable dirs for non-root runtime
RUN mkdir -p /app/data /app/.cache /app/.local/share && \
    chown -R alpen:alpen /app

COPY --from=builder /usr/local/bin/strata /usr/local/bin/strata
COPY docker/strata-checkpoint-sync/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 0555 /usr/local/bin/strata /usr/local/bin/entrypoint.sh

USER alpen

EXPOSE 8432 8080
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

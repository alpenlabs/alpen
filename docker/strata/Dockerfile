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
      build-essential ca-certificates clang libclang-dev pkg-config libssl-dev && \
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

# Optional comma-separated cargo features (e.g. "prover,sp1").
# Applied to both `cargo chef cook` and `cargo build` so the cached dependency
# layer matches the final build.
ARG CARGO_FEATURES=""

# Bring in the recipe from planner
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json \
      ${CARGO_FEATURES:+--features "$CARGO_FEATURES"}

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo build --locked --release --bin strata \
      ${CARGO_FEATURES:+--features "$CARGO_FEATURES"} && \
    install -m 0555 /app/target/release/strata /usr/local/bin/strata

# Build strata-datatool with sp1-docker-builder so the image carries its own
# datatool binary AND the byte-reproducible SP1 guest ELFs used by the
# runtime prover. sp1-docker-builder spawns a pinned Succinct toolchain
# container during compilation, so the build host needs Docker socket
# access — see the PR description for the required CI buildx config.
RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    --mount=type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
    cargo build --locked --release --bin strata-datatool \
      --features sp1-docker-builder && \
    install -m 0555 /app/target/release/strata-datatool /usr/local/bin/strata-datatool && \
    mkdir -p /app/elfs/sp1 && \
    find /app/provers/sp1 -name '*.elf' -path '*/release/*' \
      -type f -exec cp -v {} /app/elfs/sp1/ \;


# ---- runtime ----
FROM debian:13-slim AS runtime

ENV APP_HOME=/app \
    HOME=/app \
    RUST_LOG=info \
    ELF_BASE_PATH=/app/elfs/sp1

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 curl jq && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir "${APP_HOME}" --uid 10001 --shell /usr/sbin/nologin alpen

WORKDIR "${APP_HOME}"

# Ensure writable dirs for non-root runtime
RUN mkdir -p /app/data /app/.cache /app/.local/share && \
    chown -R alpen:alpen /app

COPY --from=builder /usr/local/bin/strata /usr/local/bin/strata
COPY --from=builder /usr/local/bin/strata-datatool /usr/local/bin/strata-datatool
COPY --from=builder /app/elfs /app/elfs
COPY docker/strata/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 0555 /usr/local/bin/strata /usr/local/bin/strata-datatool /usr/local/bin/entrypoint.sh

USER alpen

EXPOSE 8432
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

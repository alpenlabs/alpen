# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.92
FROM rust:${RUST_VERSION}-slim-trixie AS builder

WORKDIR /app

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang libclang-dev pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Copy toolchain + cargo manifests early for better caching
COPY rust-toolchain.toml ./
COPY Cargo.toml Cargo.lock ./

# This reads rust-toolchain.toml and installs the right toolchain/components/targets
RUN rustup toolchain install

# Now copy the rest of the repo
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo build --locked --release --bin alpen-client && \
    install -m 0555 /app/target/release/alpen-client /usr/local/bin/alpen-client


FROM debian:13-slim AS runtime

ENV APP_HOME=/app \
    HOME=/app \
    RUST_LOG=info

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir "${APP_HOME}" --uid 10001 --shell /usr/sbin/nologin alpen

WORKDIR "${APP_HOME}"

# Ensure writable dirs for non-root runtime
RUN mkdir -p /app/data /app/.cache /app/.local/share && \
    chown -R alpen:alpen /app

COPY --from=builder /usr/local/bin/alpen-client /usr/local/bin/alpen-client
COPY docker/alpen-client/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 0555 /usr/local/bin/alpen-client /usr/local/bin/entrypoint.sh

USER alpen

EXPOSE 8545 8546 30303
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

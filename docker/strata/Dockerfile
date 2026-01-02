# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.92
ARG RUST_TOOLCHAIN=nightly-2025-12-01

FROM rust:${RUST_VERSION}-slim-trixie AS builder

ARG RUST_TOOLCHAIN
WORKDIR /app

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always \
    RUSTUP_TOOLCHAIN=${RUST_TOOLCHAIN}

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang libclang-dev pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

RUN rustup toolchain install --profile minimal "${RUST_TOOLCHAIN}"

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo +"${RUST_TOOLCHAIN}" build --locked --release --bin strata && \
    install -d /out && \
    install -m 0555 /app/target/release/strata /out/strata


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

COPY --from=builder /out/strata /usr/local/bin/strata
COPY docker/strata/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 0555 /usr/local/bin/strata /usr/local/bin/entrypoint.sh

USER alpen

EXPOSE 8432
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

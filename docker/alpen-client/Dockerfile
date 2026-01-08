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

# Install cargo-chef once (cached layer)
RUN cargo install cargo-chef --locked

# ---- planner: generate recipe.json ----
FROM chef AS planner
COPY rust-toolchain.toml ./
COPY . .
RUN rustup toolchain install
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
    cargo chef cook --release --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,sharing=private,target=/app/target \
    cargo build --locked --release --bin alpen-client && \
    install -m 0555 /app/target/release/alpen-client /usr/local/bin/alpen-client


# ---- runtime ----
FROM debian:13-slim AS runtime

ENV APP_HOME=/app \
    HOME=/app \
    RUST_LOG=info

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir "${APP_HOME}" --uid 10001 --shell /usr/sbin/nologin alpen

WORKDIR "${APP_HOME}"

RUN mkdir -p /app/data /app/.cache /app/.local/share && \
    chown -R alpen:alpen /app

COPY --from=builder /usr/local/bin/alpen-client /usr/local/bin/alpen-client
COPY docker/alpen-client/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0555 /usr/local/bin/alpen-client /usr/local/bin/entrypoint.sh

USER alpen

EXPOSE 8545 8546 30303
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

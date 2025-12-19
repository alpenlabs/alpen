FROM rust:latest AS builder

WORKDIR /app

# Set environment variables for optimized release builds
ENV CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always \
    TARGET=x86_64-unknown-linux-musl

# Install system dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y \
    pkg-config \
    build-essential \
    libclang-dev

COPY . .

# Install rust toolchain
RUN rustup toolchain install

# Build dependencies in release mode
RUN cargo fetch

RUN cargo build --release --bin alpen-reth


RUN cp /app/target/release/alpen-reth /app/alpen-reth

FROM ubuntu:24.04 AS runtime
WORKDIR /app

RUN apt-get update && \
    apt-get install -y \
    curl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Copy the built binaries from the builder stage
COPY --from=builder /app/alpen-reth /usr/local/bin/alpen-reth

COPY ./docker/alpen-reth/entrypoint.sh entrypoint.sh

RUN chmod +x /app/entrypoint.sh

EXPOSE 30303 8545 8546 8551

ENTRYPOINT ["/app/entrypoint.sh"]

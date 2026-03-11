# Stage 1: Build Rust binaries
FROM ubuntu:24.04 AS builder

WORKDIR /build

# Install minimal build dependencies in one layer, clean up immediately
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates build-essential pkg-config \
    libssl-dev libclang-dev cmake protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*

# Install rustup + the exact nightly toolchain (minimal components only)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain none --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

COPY rust-toolchain.toml rust-toolchain.toml
RUN rustup show

COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY bin bin
COPY provers provers
COPY tests tests
COPY benches benches

RUN cargo build -F sequencer -F debug-utils -F test-mode \
    --bin strata --bin alpen-client --bin strata-datatool --bin strata-test-cli \
    && mkdir -p /out \
    && cp target/debug/strata target/debug/alpen-client \
       target/debug/strata-datatool target/debug/strata-test-cli /out/

# Stage 2: Runtime image with bitcoind + Python + test binaries
FROM ubuntu:24.04 AS runtime

ARG BITCOIN_VERSION=30.2
ARG TARGETARCH

# Install runtime deps + Python + uv
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates wget curl git build-essential python3-dev \
    libgmp-dev libssl-dev libssl3 python3 python3-venv \
    && rm -rf /var/lib/apt/lists/*

# Install bitcoind
RUN case "${TARGETARCH:-amd64}" in \
        amd64) ARCH="x86_64-linux-gnu" ;; \
        arm64) ARCH="aarch64-linux-gnu" ;; \
        *) echo "unsupported arch: ${TARGETARCH}" >&2; exit 1 ;; \
    esac && \
    wget -q "https://bitcoincore.org/bin/bitcoin-core-${BITCOIN_VERSION}/bitcoin-${BITCOIN_VERSION}-${ARCH}.tar.gz" && \
    tar -xzf "bitcoin-${BITCOIN_VERSION}-${ARCH}.tar.gz" && \
    install -m 0755 "bitcoin-${BITCOIN_VERSION}/bin/bitcoind" /usr/local/bin/ && \
    install -m 0755 "bitcoin-${BITCOIN_VERSION}/bin/bitcoin-cli" /usr/local/bin/ && \
    rm -rf "bitcoin-${BITCOIN_VERSION}"* && \
    apt-get purge -y wget && apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

# Install uv
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"

# Copy built binaries
COPY --from=builder /out/ /usr/local/bin/

# Copy functional test suite
WORKDIR /tests
COPY functional-tests-new/pyproject.toml functional-tests-new/uv.lock ./
RUN uv sync

COPY functional-tests-new/entry.py ./
COPY functional-tests-new/common ./common
COPY functional-tests-new/envconfigs ./envconfigs
COPY functional-tests-new/factories ./factories
COPY functional-tests-new/tests ./tests

ENV RUST_BACKTRACE=1
ENV RUST_LOG="debug,sled=warn,hyper=warn,soketto=warn,jsonrpsee-server=warn,mio=warn"

ENTRYPOINT ["uv", "run", "entry.py"]

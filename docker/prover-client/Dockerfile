FROM --platform=linux/amd64 rust:latest AS builder

WORKDIR /app
# Set environment variables for optimized release builds
ENV CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=always

# Install system dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y \
    pkg-config \
    ca-certificates \
    clang \
    libssl-dev \
    git \
    dialog \
    xz-utils \
    build-essential \
    curl \
    libclang-dev \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

COPY . .

# Install rust toolchain
RUN rustup toolchain install

# Accept an argument for Cargo features
ARG PROVER_FEATURES=""

# Build dependencies in release mode
RUN cargo build --release --bin strata-prover-client -F ${PROVER_FEATURES}

RUN cp /app/target/release/strata-prover-client /app/strata-prover-client

FROM --platform=linux/amd64 ubuntu:24.04 AS runtime
WORKDIR /app

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y \
    curl \
    iproute2 \
    net-tools \
    libssl-dev \
    libffi-dev \
    software-properties-common && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Copy the built binaries and the entrypoint script from the builder stage
COPY --from=builder /app/strata-prover-client /usr/local/bin/strata-prover-client
COPY ./docker/prover-client/entrypoint.sh entrypoint.sh
COPY ./docker/prover-client/elfs /app/elfs

EXPOSE 9851

# Make the entrypoint script executable
RUN chmod +x entrypoint.sh

# Set the entrypoint
ENTRYPOINT ["/app/entrypoint.sh"]

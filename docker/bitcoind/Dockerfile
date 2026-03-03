FROM debian:bookworm-slim

ARG BITCOIN_VERSION=30.2
ARG TARGETARCH

WORKDIR /app

# Fetch pre-built bitcoind + bitcoin-cli from bitcoincore.org
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates wget && \
    case "${TARGETARCH}" in \
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

# Create a directory for Bitcoin data
RUN mkdir -p /root/.bitcoin

COPY bcli.sh bcli.sh
RUN chmod +x /app/bcli.sh

# regtest RPC/P2P + signet RPC/P2P
EXPOSE 18443 18444 38332 38333

# this should be mounted so that we can set args more easily
ENTRYPOINT ["/app/entrypoint.sh"]

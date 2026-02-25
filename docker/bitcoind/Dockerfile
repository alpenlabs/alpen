FROM ubuntu:24.04 AS build

WORKDIR /build

RUN apt-get update && \
    apt-get install -y build-essential cmake pkg-config python3 \
    libevent-dev libboost-dev libsqlite3-dev libssl-dev git

RUN git clone -b 'v29.0' --single-branch --depth 1 https://github.com/bitcoin/bitcoin.git

COPY configurable_blocktime_patch.diff configurable_blocktime_patch.diff
RUN cd bitcoin && git apply /build/configurable_blocktime_patch.diff

RUN cd bitcoin && \
    cmake -B build && \
    cmake --build build -j$(($(nproc) - 1)) && \
    cmake --install build

FROM ubuntu:24.04 AS runtime

WORKDIR /app

RUN apt-get update && \
    apt-get install -y libevent-dev libsqlite3-dev libssl-dev && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local/bin/bitcoind /usr/local/bin/bitcoind
COPY --from=build /usr/local/bin/bitcoin-cli /usr/local/bin/bitcoin-cli

RUN mkdir -p /root/.bitcoin

COPY bcli.sh bcli.sh
RUN chmod +x /app/bcli.sh

EXPOSE 18443 18444

ENTRYPOINT ["/app/entrypoint.sh"]

export RUST_LOG=debug,sled=info,hyper=warn,soketto=warn,jsonrpsee-server=warn,mio=warn,bitcoind-async-client::client=warn,trie=warn
export NO_COLOR=1
# shellcheck disable=2155
export PATH="$PATH:$(realpath ../target/release)"
export RUST_BACKTRACE=1
export LOG_LEVEL=info

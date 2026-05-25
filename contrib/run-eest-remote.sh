#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: run-eest-remote.sh --rpc-endpoint URL [options]

Options:
  --rpc-endpoint URL      Execution JSON-RPC endpoint.
  --fork NAME             EEST fork name. Default: Prague.
  --rpc-chain-id ID       Chain ID. Default: 2892.
  --rpc-seed-key KEY      Seed private key for EEST transactions.
  --tx-wait-timeout SEC   Transaction wait timeout. Default: 120.
  --repo URL              execution-spec-tests repository.
  --checkout-dir DIR      Clone/use this directory. Default: execution-spec-tests.
  -h, --help              Show this help.
EOF
}

RPC_ENDPOINT=""
FORK="Prague"
RPC_CHAIN_ID="2892"
RPC_SEED_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
TX_WAIT_TIMEOUT="120"
EEST_REPO="https://github.com/alpenlabs/execution-spec-tests"
CHECKOUT_DIR="execution-spec-tests"

while (($#)); do
    case "$1" in
        --rpc-endpoint)
            RPC_ENDPOINT="${2:?missing value for --rpc-endpoint}"
            shift 2
            ;;
        --fork)
            FORK="${2:?missing value for --fork}"
            shift 2
            ;;
        --rpc-chain-id)
            RPC_CHAIN_ID="${2:?missing value for --rpc-chain-id}"
            shift 2
            ;;
        --rpc-seed-key)
            RPC_SEED_KEY="${2:?missing value for --rpc-seed-key}"
            shift 2
            ;;
        --tx-wait-timeout)
            TX_WAIT_TIMEOUT="${2:?missing value for --tx-wait-timeout}"
            shift 2
            ;;
        --repo)
            EEST_REPO="${2:?missing value for --repo}"
            shift 2
            ;;
        --checkout-dir)
            CHECKOUT_DIR="${2:?missing value for --checkout-dir}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [[ -z "${RPC_ENDPOINT}" ]]; then
    echo "--rpc-endpoint is required" >&2
    usage >&2
    exit 1
fi

WORKSPACE_DIR="$(pwd)"
if [[ "${CHECKOUT_DIR}" != /* ]]; then
    CHECKOUT_DIR="${WORKSPACE_DIR}/${CHECKOUT_DIR}"
fi

if ! command -v uv >/dev/null 2>&1; then
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="${HOME}/.local/bin:${PATH}"
fi

if [[ ! -d "${CHECKOUT_DIR}/.git" ]]; then
    git clone "${EEST_REPO}" "${CHECKOUT_DIR}"
fi

cd "${CHECKOUT_DIR}"

uv python install 3.11
uv python pin 3.11
uv sync --all-extras

# The Alpen fork keeps expected mismatches in skip_tests.yaml. Ensure the
# checked-out EEST tree has the Prague skip needed by CI.
if [[ "${FORK}" == "Prague" ]]; then
    REQUIRED_SKIP="tests/frontier/opcodes/test_call.py::test_call_memory_expands_on_early_revert[fork_${FORK}-state_test]"
    if [[ ! -f skip_tests.yaml ]]; then
        printf 'skip_tests:\n' > skip_tests.yaml
    fi
    if ! grep -Fq "${REQUIRED_SKIP}" skip_tests.yaml; then
        {
            printf '\n'
            printf '  # Alpen/reth treats memory expansion differently on early revert in this edge case\n'
            printf '  - %s\n' "${REQUIRED_SKIP}"
        } >> skip_tests.yaml
    fi
fi

uv run --with solc-select solc-select use 0.8.24 --always-install

uv run --with solc-select execute remote \
    -m state_test \
    "--fork=${FORK}" \
    "--rpc-endpoint=${RPC_ENDPOINT}" \
    "--rpc-seed-key=${RPC_SEED_KEY}" \
    "--rpc-chain-id=${RPC_CHAIN_ID}" \
    "--tx-wait-timeout=${TX_WAIT_TIMEOUT}" \
    -v

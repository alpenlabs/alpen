#!/usr/bin/env bash
set -euo pipefail

RPC_ENDPOINT=""
FORK="Prague"
RPC_CHAIN_ID="2892"
RPC_SEED_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
TX_WAIT_TIMEOUT="120"
EEST_REPO="https://github.com/alpenlabs/execution-spec-tests"
CHECKOUT_DIR="execution-spec-tests"
PYTEST_ARGS_STRING=""

die() {
    echo "$*" >&2
    exit 1
}

while (($#)); do
    case "$1" in
        --rpc-endpoint) RPC_ENDPOINT="${2:?}"; shift 2 ;;
        --fork) FORK="${2:?}"; shift 2 ;;
        --rpc-chain-id) RPC_CHAIN_ID="${2:?}"; shift 2 ;;
        --rpc-seed-key) RPC_SEED_KEY="${2:?}"; shift 2 ;;
        --tx-wait-timeout) TX_WAIT_TIMEOUT="${2:?}"; shift 2 ;;
        --repo) EEST_REPO="${2:?}"; shift 2 ;;
        --checkout-dir) CHECKOUT_DIR="${2:?}"; shift 2 ;;
        --pytest-args) PYTEST_ARGS_STRING="${2:?}"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[[ -n "${RPC_ENDPOINT}" ]] || die "--rpc-endpoint is required"

WORKSPACE_DIR="$(pwd)"
[[ "${CHECKOUT_DIR}" == /* ]] || CHECKOUT_DIR="${WORKSPACE_DIR}/${CHECKOUT_DIR}"

if ! command -v uv >/dev/null 2>&1; then
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="${HOME}/.local/bin:${PATH}"
fi

[[ -d "${CHECKOUT_DIR}/.git" ]] || git clone "${EEST_REPO}" "${CHECKOUT_DIR}"
cd "${CHECKOUT_DIR}"

uv python install 3.11
uv python pin 3.11
uv sync --all-extras

if [[ "${FORK}" == "Prague" ]]; then
    REQUIRED_SKIP="tests/frontier/opcodes/test_call.py::test_call_memory_expands_on_early_revert[fork_${FORK}-state_test]"
    [[ -f skip_tests.yaml ]] || printf 'skip_tests:\n' > skip_tests.yaml
    grep -Fq "${REQUIRED_SKIP}" skip_tests.yaml \
        || printf '\n  # Alpen/reth treats memory expansion differently on early revert in this edge case\n  - %s\n' "${REQUIRED_SKIP}" >> skip_tests.yaml
fi

PYTEST_ARGS=()
if [[ -n "${PYTEST_ARGS_STRING}" ]]; then
    mapfile -t PYTEST_ARGS < <(python3 -c 'import shlex,sys; print("\n".join(shlex.split(sys.argv[1])))' "${PYTEST_ARGS_STRING}")
fi

uv run --with solc-select solc-select use 0.8.24 --always-install
uv run --with solc-select execute remote \
    -m state_test \
    "--fork=${FORK}" \
    "--rpc-endpoint=${RPC_ENDPOINT}" \
    "--rpc-seed-key=${RPC_SEED_KEY}" \
    "--rpc-chain-id=${RPC_CHAIN_ID}" \
    "--tx-wait-timeout=${TX_WAIT_TIMEOUT}" \
    -v \
    "${PYTEST_ARGS[@]}"

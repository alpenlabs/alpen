#!/usr/bin/env bash
set -euo pipefail

# Generate params from templates using prebuilt datatool image.
#
# Required env vars:
#   DEPLOY_ENV            - environment (staging-v2, testnet-prod)
#   COMMIT                - alpen commit short SHA (datatool image tag)
#   BTC_RPC_URL           - bitcoin RPC endpoint
#   BTC_RPC_USER          - bitcoin RPC username
#   BTC_RPC_PASSWORD      - bitcoin RPC password
#   GENESIS_L1_HEIGHT     - genesis L1 block height
#   CHAIN_CONFIG          - path to chainspec JSON
#   NETWORK               - bitcoin network (default: signet)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ECR_REGISTRY="496607027995.dkr.ecr.us-east-1.amazonaws.com"
DT_PLATFORM="linux/amd64"
NETWORK="${NETWORK:-signet}"

for var in DEPLOY_ENV COMMIT BTC_RPC_URL BTC_RPC_USER BTC_RPC_PASSWORD GENESIS_L1_HEIGHT CHAIN_CONFIG; do
    if [ -z "${!var:-}" ]; then
        echo "Missing required env var: ${var}" >&2
        exit 1
    fi
done

TEMPLATE_DIR="${SCRIPT_DIR}/templates/${DEPLOY_ENV}"
OUTPUT_DIR="${SCRIPT_DIR}/out/${DEPLOY_ENV}"

if [ ! -d "${TEMPLATE_DIR}" ]; then
    echo "No templates for '${DEPLOY_ENV}'. Available: $(ls "${SCRIPT_DIR}/templates/")" >&2
    exit 1
fi

DT_IMG="${ECR_REGISTRY}/strata/strata-datatool:${COMMIT}"
CHAIN_CONFIG_ABS="$(cd "$(dirname "${CHAIN_CONFIG}")" && pwd)/$(basename "${CHAIN_CONFIG}")"

mkdir -p "${OUTPUT_DIR}"
WORK_DIR=$(mktemp -d)
chmod 777 "${WORK_DIR}"
trap 'rm -rf "${WORK_DIR}"' EXIT

json_value() {
    local file="$1"
    local path="$2"
    python3 - "$file" "$path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    value = json.load(f)

for key in sys.argv[2].split("."):
    value = value[key]

print("" if value is None else value)
PY
}

echo "=== Reading keys from templates ==="
python3 "${SCRIPT_DIR}/params-helper.py" extract-keys \
    --template-dir "${TEMPLATE_DIR}" \
    --output-dir "${WORK_DIR}"
SAFE_HARBOUR=$(tr -d '[:space:]' < "${WORK_DIR}/safe-harbour.txt")
EE_ACCOUNT_ID=$(tr -d '[:space:]' < "${WORK_DIR}/ee-account-id.txt")
EE_PARAMS_TEMPLATE="${TEMPLATE_DIR}/ee-params.json"
BRIDGE_DENOMINATION_SATS=$(json_value "${EE_PARAMS_TEMPLATE}" "bridge_params.denomination")
MAX_WITHDRAWAL_AMOUNT_SATS=$(json_value "${EE_PARAMS_TEMPLATE}" "bridge_params.max_withdrawal_amount")
MAX_WITHDRAWAL_DESCRIPTOR_LEN=$(json_value "${EE_PARAMS_TEMPLATE}" "bridge_params.max_withdrawal_descriptor_len")

echo ""
echo "=== Step 1: Generate raw params via datatool ==="

# Run datatool with common docker + bitcoin args.
# Usage: run_datatool [extra-volume-flags...] -- <subcommand> [args...]
run_datatool() {
    local vols=(-v "${WORK_DIR}:/out")
    while [ "${1:-}" != "--" ]; do vols+=(-v "$1"); shift; done
    shift # consume --
    docker run --rm --platform "${DT_PLATFORM}" --network host \
        "${vols[@]}" \
        "${DT_IMG}" -b "${NETWORK}" \
        --bitcoin-rpc-url "${BTC_RPC_URL}" \
        --bitcoin-rpc-user "${BTC_RPC_USER}" \
        --bitcoin-rpc-password "${BTC_RPC_PASSWORD}" \
        "$@"
}

run_datatool "${CHAIN_CONFIG_ABS}:/app/chain.json:ro" -- \
    gen-ee-params \
    --alpen-chain-config /app/chain.json \
    --account-id "${EE_ACCOUNT_ID}" \
    --bridge-denomination-sats "${BRIDGE_DENOMINATION_SATS}" \
    ${MAX_WITHDRAWAL_AMOUNT_SATS:+--max-withdrawal-amount-sats "${MAX_WITHDRAWAL_AMOUNT_SATS}"} \
    --max-withdrawal-descriptor-len "${MAX_WITHDRAWAL_DESCRIPTOR_LEN}" \
    -o /out/ee-params-raw.json
echo "  ee-params-raw.json generated"

run_datatool "${CHAIN_CONFIG_ABS}:/app/chain.json:ro" -- \
    gen-ol-params \
    --alpen-predicate sp1-groth16 \
    --alpen-chain-config /app/chain.json \
    --ee-params /out/ee-params-raw.json \
    --genesis-l1-height "${GENESIS_L1_HEIGHT}" \
    -o /out/ol-params-raw.json
echo "  ol-params-raw.json generated"

# The ASM checkpoint sequencer_predicate is static (from the template); the raw
# generation only needs operators + safe-harbour, so no -s/sequencer key here.
run_datatool -- \
    gen-asm-params \
    -B /out/op-pks.txt \
    --checkpoint-predicate sp1-groth16 \
    --ol-params /out/ol-params-raw.json \
    --genesis-l1-height "${GENESIS_L1_HEIGHT}" \
    --safe-harbour-address "${SAFE_HARBOUR}" \
    -o /out/asm-params-raw.json
echo "  asm-params-raw.json generated"

# Verify raw files were actually produced
for f in ee-params-raw.json ol-params-raw.json asm-params-raw.json; do
    if [ ! -s "${WORK_DIR}/${f}" ]; then
        echo "ERROR: datatool did not produce ${f} (check BTC RPC connectivity)" >&2
        exit 1
    fi
done

echo ""
echo "=== Step 2: Merge dynamic values from raw into templates ==="

python3 "${SCRIPT_DIR}/params-helper.py" merge \
    --raw-dir "${WORK_DIR}" \
    --template-dir "${TEMPLATE_DIR}" \
    --output-dir "${OUTPUT_DIR}"

echo ""
echo "=== Output ==="
ls -la "${OUTPUT_DIR}"/*.json
echo ""
echo "Done. Params in ${OUTPUT_DIR}/"

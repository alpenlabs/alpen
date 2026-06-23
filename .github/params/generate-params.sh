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

echo "=== Reading keys from templates ==="
python3 "${SCRIPT_DIR}/params-helper.py" extract-keys \
    --template-dir "${TEMPLATE_DIR}" \
    --output-dir "${WORK_DIR}"
SAFE_HARBOUR=$(tr -d '[:space:]' < "${WORK_DIR}/safe-harbour.txt")

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
    gen-ol-params \
    --alpen-predicate sp1-groth16 \
    --alpen-chain-config /app/chain.json \
    --genesis-l1-height "${GENESIS_L1_HEIGHT}" \
    -o /out/ol-params-raw.json
echo "  ol-params-raw.json generated"

# Align the raw ol-params bridge denomination with the deployment template BEFORE
# gen-asm-params computes genesis_ol_blkid. The OL genesis STF hashes bridge_params
# (denomination + cap) into the genesis block, so genesis_ol_blkid is
# denomination-dependent (crates/ol/genesis: execute_and_complete_block consumes
# params.bridge_params()). gen-ol-params has no denomination flag and emits the
# datatool default; without this patch a template denomination != that default
# would make the node's runtime genesis diverge from the asm-params
# genesis_ol_blkid and bootstrap would fail. Patch the raw (used for
# genesis_ol_blkid) and pass the same value to --deposit-sats (Bridge.denomination)
# so ol-params, genesis_ol_blkid, and asm Bridge.denomination all agree.
# Aligned copy goes to a new file (not an in-place patch): the raw is written by
# datatool as root in docker and is not writable by the host runner user.
DEPOSIT_SATS=$(python3 "${SCRIPT_DIR}/params-helper.py" align-denomination \
    --raw "${WORK_DIR}/ol-params-raw.json" \
    --template "${TEMPLATE_DIR}/ol-params.json" \
    --out "${WORK_DIR}/ol-params-aligned.json")
echo "  ol-params bridge denomination aligned to template: ${DEPOSIT_SATS} sats"

# The ASM checkpoint sequencer_predicate is static (from the template); the raw
# generation only needs operators + safe-harbour, so no -s/sequencer key here.
# Use the denomination-aligned ol-params so genesis_ol_blkid matches the deployed
# ol-params (which carries the template denomination).
run_datatool -- \
    gen-asm-params \
    -B /out/op-pks.txt \
    --checkpoint-predicate sp1-groth16 \
    --ol-params /out/ol-params-aligned.json \
    --genesis-l1-height "${GENESIS_L1_HEIGHT}" \
    --safe-harbour-address "${SAFE_HARBOUR}" \
    --deposit-sats "${DEPOSIT_SATS}" \
    -o /out/asm-params-raw.json
echo "  asm-params-raw.json generated"

# Verify raw files were actually produced
for f in ol-params-raw.json asm-params-raw.json; do
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

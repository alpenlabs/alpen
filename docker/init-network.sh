#!/usr/bin/env bash
set -euo pipefail

# Generate keys and params for the Alpen network stack (OL + ASM).
# Uses datatool for params generation instead of hardcoded JSON.
#
# Usage:
#   ./init-network.sh <datatool_path>
#   ./init-network.sh --sequencer <datatool_path>
#   ./init-network.sh --sequencer --base-only <datatool_path>
#   ./init-network.sh --sequencer --asm-only <datatool_path> --checkpoint-predicate-file <path>
#   ./init-network.sh --fullnode <datatool_path> --params-dir <path>
#   BITCOIN_NETWORK=signet GENESIS_L1_HEIGHT=200000 ./init-network.sh <datatool_path>
#
# When BITCOIND_RPC_URL is set, the script fetches the real L1 anchor from
# the Bitcoin node via `datatool gen-l1-anchor`. Without it, a placeholder L1
# anchor is written from network-specific genesis values. The node consumes the
# anchor as-is (there is no runtime patching), so the placeholder is only correct
# for regtest at genesis height 0 (the regtest genesis block); any other
# network/height needs BITCOIND_RPC_*.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BITCOIN_NETWORK="${BITCOIN_NETWORK:-regtest}"
GENESIS_L1_HEIGHT="${GENESIS_L1_HEIGHT:-0}"
BITCOIND_RPC_URL="${BITCOIND_RPC_URL:-${BITCOIND_RPC_URL:-}}"
BITCOIND_RPC_USER="${BITCOIND_RPC_USER:-${BITCOIND_RPC_USER:-}}"
BITCOIND_RPC_PASSWORD="${BITCOIND_RPC_PASSWORD:-${BITCOIND_RPC_PASSWORD:-}}"
SAFE_HARBOUR_ADDRESS="${SAFE_HARBOUR_ADDRESS:-}"
CHECKPOINT_PREDICATE_FILE="${CHECKPOINT_PREDICATE_FILE:-}"

MODE="sequencer"
PHASE="all"
PARAMS_DIR=""
DATATOOL_PATH=""

while [ $# -gt 0 ]; do
    case "$1" in
        --sequencer)
            MODE="sequencer"
            shift
            ;;
        --fullnode)
            MODE="fullnode"
            shift
            ;;
        --base-only)
            if [ "${PHASE}" != "all" ]; then
                echo "error: --base-only and --asm-only are mutually exclusive" >&2
                exit 1
            fi
            PHASE="base"
            shift
            ;;
        --asm-only)
            if [ "${PHASE}" != "all" ]; then
                echo "error: --base-only and --asm-only are mutually exclusive" >&2
                exit 1
            fi
            PHASE="asm"
            shift
            ;;
        --params-dir)
            PARAMS_DIR="$2"
            shift 2
            ;;
        --checkpoint-predicate-file)
            CHECKPOINT_PREDICATE_FILE="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--sequencer|--fullnode] [--base-only|--asm-only] <datatool_path> [options]"
            echo ""
            echo "Modes:"
            echo "  --sequencer  Generate sequencer keys and params (default)"
            echo "  --fullnode   Validate and copy params from --params-dir"
            echo ""
            echo "Options:"
            echo "  --base-only   In sequencer mode, generate keys, L1 anchor, and OL params only"
            echo "  --asm-only    In sequencer mode, generate ASM params from existing base artifacts"
            echo "  --params-dir <dir>  Directory with existing params (required for --fullnode)"
            echo "  --checkpoint-predicate-file <path>  ASM checkpoint predicate metadata"
            echo ""
            echo "Environment:"
            echo "  BITCOIN_NETWORK       regtest (default) or signet"
            echo "  GENESIS_L1_HEIGHT     L1 block height for genesis (default: 0)"
            echo "  BITCOIND_RPC_URL       Bitcoin RPC URL (enables fetching real L1 anchor)"
            echo "  BITCOIND_RPC_USER      Bitcoin RPC username"
            echo "  BITCOIND_RPC_PASSWORD  Bitcoin RPC password"
            echo "  GENESIS_ACCOUNTS       path to the genesis snark accounts JSON from alpen-ee"
            echo "  BRIDGE_DENOMINATION_SATS           bridge denomination in satoshis"
            echo "  MAX_WITHDRAWAL_AMOUNT_SATS         optional maximum withdrawal amount in satoshis"
            echo "  MAX_WITHDRAWAL_DESCRIPTOR_LEN      maximum withdrawal BOSD descriptor length"
            echo "  OUTPUT_DIR            output directory (default: ./configs/generated)"
            exit 0
            ;;
        -*)
            echo "error: unknown option: $1" >&2
            exit 1
            ;;
        *)
            if [ -z "${DATATOOL_PATH}" ]; then
                DATATOOL_PATH="$1"
            else
                echo "error: unexpected argument: $1" >&2
                exit 1
            fi
            shift
            ;;
    esac
done

if [ -z "${DATATOOL_PATH}" ]; then
    echo "error: datatool path required. usage: $0 [--sequencer|--fullnode] <datatool_path>" >&2
    exit 1
fi

if [ ! -x "${DATATOOL_PATH}" ]; then
    echo "error: datatool not found or not executable: ${DATATOOL_PATH}" >&2
    exit 1
fi

if [ "${MODE}" = "fullnode" ] && [ -z "${PARAMS_DIR}" ]; then
    echo "error: --params-dir is required for fullnode mode" >&2
    exit 1
fi

if [ "${MODE}" = "fullnode" ] && [ "${PHASE}" != "all" ]; then
    echo "error: --base-only and --asm-only are only valid in sequencer mode" >&2
    exit 1
fi

if [ -n "${PARAMS_DIR}" ] && [ ! -d "${PARAMS_DIR}" ]; then
    echo "error: params directory not found: ${PARAMS_DIR}" >&2
    exit 1
fi

OUTPUT_DIR="${OUTPUT_DIR:-${SCRIPT_DIR}/configs/generated}"
GENESIS_ACCOUNTS="${GENESIS_ACCOUNTS:-${SCRIPT_DIR}/../.github/fixtures/alpen-ee-genesis.dev.json}"

case "${BITCOIN_NETWORK}" in
    regtest)
        GENESIS_BLKID="0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"
        L1_NEXT_TARGET=545259519
        L1_EPOCH_START_TIMESTAMP=1296688602
        ;;
    signet)
        GENESIS_BLKID="00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6"
        L1_NEXT_TARGET=503543726
        L1_EPOCH_START_TIMESTAMP=1598918400
        ;;
    *)
        echo "error: unsupported BITCOIN_NETWORK=${BITCOIN_NETWORK} (use regtest or signet)" >&2
        exit 1
        ;;
esac

mkdir -p "${OUTPUT_DIR}"

if [ "${MODE}" = "sequencer" ]; then
    echo "mode: sequencer"

    SEQ_ROOT_KEY="${OUTPUT_DIR}/sequencer.key"
    OPERATOR_KEY="${OUTPUT_DIR}/operator.key"
    L1_ANCHOR="${OUTPUT_DIR}/l1-anchor.json"
    OL_PARAMS="${OUTPUT_DIR}/ol-params.json"
    ASM_PARAMS="${OUTPUT_DIR}/asm-params.json"

    if [ "${PHASE}" != "asm" ]; then
        if [ ! -f "${SEQ_ROOT_KEY}" ]; then
            "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genxpriv "${SEQ_ROOT_KEY}"
            echo "generated ${SEQ_ROOT_KEY}"
        fi

        if [ ! -f "${OPERATOR_KEY}" ]; then
            "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genxpriv "${OPERATOR_KEY}"
            echo "generated ${OPERATOR_KEY}"
        fi

        if [ ! -f "${L1_ANCHOR}" ]; then
            if [ -n "${BITCOIND_RPC_URL}" ] && [ -n "${BITCOIND_RPC_USER}" ] && [ -n "${BITCOIND_RPC_PASSWORD}" ]; then
                # Fetch real L1 anchor from Bitcoin node — produces correct values for
                # all fields (next_target, epoch_start_timestamp, network).
                echo "fetching genesis L1 anchor from ${BITCOIND_RPC_URL} at height ${GENESIS_L1_HEIGHT}..."
                "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
                    --bitcoin-rpc-url "${BITCOIND_RPC_URL}" \
                    --bitcoin-rpc-user "${BITCOIND_RPC_USER}" \
                    --bitcoin-rpc-password "${BITCOIND_RPC_PASSWORD}" \
                    gen-l1-anchor \
                    -g "${GENESIS_L1_HEIGHT}" \
                    -o "${L1_ANCHOR}"
                echo "generated ${L1_ANCHOR} (from Bitcoin RPC)"
            else
                # No RPC available — write a placeholder L1 anchor from network-specific
                # genesis block values. The node consumes the anchor as-is (no runtime
                # patching), so this is only correct for regtest at height 0 (the regtest
                # genesis block); any non-zero genesis height needs BITCOIN_RPC_* for a
                # correct blkid and next_target.
                if [ "${GENESIS_L1_HEIGHT}" != "0" ]; then
                    echo "warning: generating placeholder L1 anchor at height ${GENESIS_L1_HEIGHT} without Bitcoin RPC;" >&2
                    echo "         blkid and next_target will not match the real chain." >&2
                    echo "         Set BITCOIND_RPC_URL, BITCOIND_RPC_USER, BITCOIND_RPC_PASSWORD for correct values." >&2
                fi
                cat > "${L1_ANCHOR}" <<GEOF
{
  "block": {
    "height": ${GENESIS_L1_HEIGHT},
    "blkid": "${GENESIS_BLKID}"
  },
  "next_target": ${L1_NEXT_TARGET},
  "epoch_start_timestamp": ${L1_EPOCH_START_TIMESTAMP},
  "network": "${BITCOIN_NETWORK}"
}
GEOF
                echo "generated ${L1_ANCHOR} (placeholder)"
            fi
        fi

        if [ ! -f "${OL_PARAMS}" ]; then
            : "${BRIDGE_DENOMINATION_SATS:?BRIDGE_DENOMINATION_SATS is required when generating ol-params.json}"
            : "${MAX_WITHDRAWAL_DESCRIPTOR_LEN:?MAX_WITHDRAWAL_DESCRIPTOR_LEN is required}"
            "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
                gen-ol-params \
                -o "${OL_PARAMS}" \
                -g "${GENESIS_L1_HEIGHT}" \
                --l1-anchor-file "${L1_ANCHOR}" \
                --genesis-accounts "${GENESIS_ACCOUNTS}" \
                --bridge-denomination-sats "${BRIDGE_DENOMINATION_SATS}" \
                ${MAX_WITHDRAWAL_AMOUNT_SATS:+--max-withdrawal-amount-sats "$MAX_WITHDRAWAL_AMOUNT_SATS"} \
                --max-withdrawal-descriptor-len "${MAX_WITHDRAWAL_DESCRIPTOR_LEN}"
            echo "generated ${OL_PARAMS}"
        fi
    fi

    if [ "${PHASE}" != "base" ]; then
        for f in "${SEQ_ROOT_KEY}" "${OPERATOR_KEY}" "${L1_ANCHOR}" "${OL_PARAMS}"; do
            if [ ! -f "${f}" ]; then
                echo "error: missing required base artifact for ASM params: ${f}" >&2
                exit 1
            fi
        done

        OPERATOR_PK=$("${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genoppubkey -f "${OPERATOR_KEY}")
        SEQ_PK=$("${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genseqpubkey -f "${SEQ_ROOT_KEY}")

        : "${CHECKPOINT_PREDICATE_FILE:?CHECKPOINT_PREDICATE_FILE is required when generating asm-params.json}"
        : "${SAFE_HARBOUR_ADDRESS:?SAFE_HARBOUR_ADDRESS is required when generating asm-params.json: provide a P2TR BOSD descriptor for the bridge emergency sweep address}"
        if [ ! -f "${ASM_PARAMS}" ]; then
            "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
                gen-asm-params \
                -o "${ASM_PARAMS}" \
                -n ALPN \
                -s "${SEQ_PK}" \
                -b "${OPERATOR_PK}" \
                -g "${GENESIS_L1_HEIGHT}" \
                --l1-anchor-file "${L1_ANCHOR}" \
                --ol-params "${OL_PARAMS}" \
                --safe-harbour-address "${SAFE_HARBOUR_ADDRESS}" \
                --checkpoint-predicate-file "${CHECKPOINT_PREDICATE_FILE}"
            echo "generated ${ASM_PARAMS}"
        fi

        echo "sequencer pubkey: ${SEQ_PK}"
    fi

    echo "network: ${BITCOIN_NETWORK}"

elif [ "${MODE}" = "fullnode" ]; then
    echo "mode: fullnode"

    for f in ol-params.json asm-params.json; do
        if [ ! -f "${PARAMS_DIR}/${f}" ]; then
            echo "error: missing ${f} in ${PARAMS_DIR}" >&2
            exit 1
        fi
    done

    if [ "$(realpath "${PARAMS_DIR}")" != "$(realpath "${OUTPUT_DIR}")" ]; then
        for f in ol-params.json asm-params.json; do
            cp "${PARAMS_DIR}/${f}" "${OUTPUT_DIR}/${f}"
        done
        echo "copied params from ${PARAMS_DIR}"
    fi

    echo "network: ${BITCOIN_NETWORK}"
fi

#!/usr/bin/env bash
set -euo pipefail

# Generate keys and params for the full Alpen network stack (OL + EE + ASM).
# Uses datatool for params generation instead of hardcoded JSON.
#
# Usage:
#   ./init-network.sh <datatool_path>
#   ./init-network.sh --sequencer <datatool_path>
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
SAFE_HARBOUR_ADDRESS="${SAFE_HARBOUR_ADDRESS:?SAFE_HARBOUR_ADDRESS is required: provide a P2TR BOSD descriptor for the bridge emergency sweep address}"

MODE="sequencer"
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
        --params-dir)
            PARAMS_DIR="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--sequencer|--fullnode] <datatool_path> [--params-dir <dir>]"
            echo ""
            echo "Modes:"
            echo "  --sequencer  Generate all keys and params (default)"
            echo "  --fullnode   Generate P2P key only, read params from --params-dir"
            echo ""
            echo "Options:"
            echo "  --params-dir <dir>  Directory with existing params (required for --fullnode)"
            echo ""
            echo "Environment:"
            echo "  BITCOIN_NETWORK       regtest (default) or signet"
            echo "  GENESIS_L1_HEIGHT     L1 block height for genesis (default: 0)"
            echo "  BITCOIND_RPC_URL       Bitcoin RPC URL (enables fetching real L1 anchor)"
            echo "  BITCOIND_RPC_USER      Bitcoin RPC username"
            echo "  BITCOIND_RPC_PASSWORD  Bitcoin RPC password"
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

if [ -n "${PARAMS_DIR}" ] && [ ! -d "${PARAMS_DIR}" ]; then
    echo "error: params directory not found: ${PARAMS_DIR}" >&2
    exit 1
fi

OUTPUT_DIR="${OUTPUT_DIR:-${SCRIPT_DIR}/configs/generated}"

case "${BITCOIN_NETWORK}" in
    regtest)
        GENESIS_BLKID="0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"
        DEFAULT_RPC_PORT=18443
        L1_NEXT_TARGET=545259519
        L1_EPOCH_START_TIMESTAMP=1296688602
        ;;
    signet)
        GENESIS_BLKID="00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6"
        DEFAULT_RPC_PORT=38332
        L1_NEXT_TARGET=503543726
        L1_EPOCH_START_TIMESTAMP=1598918400
        ;;
    *)
        echo "error: unsupported BITCOIN_NETWORK=${BITCOIN_NETWORK} (use regtest or signet)" >&2
        exit 1
        ;;
esac

PYTHON=""
for candidate in python3 python3.12 python3.11 python3.10 python; do
    if command -v "${candidate}" &>/dev/null && "${candidate}" -c "import coincurve" 2>/dev/null; then
        PYTHON="${candidate}"
        break
    fi
done

if [ -z "${PYTHON}" ]; then
    echo "error: no python with 'coincurve' found. install: pip install coincurve" >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

generate_secret_key() {
    od -An -tx1 -N32 /dev/urandom | tr -d ' \n'
}

derive_schnorr_pubkey() {
    local privkey_hex="$1"
    echo -n "${privkey_hex}" | "${PYTHON}" -c "
import coincurve, sys
pk = coincurve.PublicKey.from_secret(bytes.fromhex(sys.stdin.read()))
sys.stdout.write(pk.format(compressed=True)[1:].hex())
"
}

derive_enode_pubkey() {
    local privkey_hex="$1"
    echo -n "${privkey_hex}" | "${PYTHON}" -c "
import coincurve, sys
pk = coincurve.PublicKey.from_secret(bytes.fromhex(sys.stdin.read()))
sys.stdout.write(pk.format(compressed=False)[1:].hex())
"
}

bridge_runtime_env() {
    local ol_params="$1"
    "${PYTHON}" - "${ol_params}" <<'PY'
import json
import sys

MAX_U64_MINUS_ONE = 2**64 - 2

with open(sys.argv[1]) as f:
    ol_params = json.load(f)

bridge_params = ol_params["bridge_params"]
denomination = bridge_params["denomination"]
max_withdrawal_amount = bridge_params.get("max_withdrawal_amount")

if max_withdrawal_amount is None:
    max_withdrawal_amount = (MAX_U64_MINUS_ONE // denomination) * denomination

print(f"BRIDGE_DENOMINATION={denomination}")
print(f"MAX_WITHDRAWAL_AMOUNT={max_withdrawal_amount}")
PY
}

generate_key_file() {
    local filepath="$1"
    if [ -f "${filepath}" ]; then
        return
    fi
    generate_secret_key > "${filepath}"
}

if [ "${MODE}" = "sequencer" ]; then
    echo "mode: sequencer"

    SCHNORR_KEY="${OUTPUT_DIR}/sequencer-schnorr.hex"
    generate_key_file "${SCHNORR_KEY}"
    SCHNORR_PRIVKEY=$(cat "${SCHNORR_KEY}")
    SCHNORR_PUBKEY=$(derive_schnorr_pubkey "${SCHNORR_PRIVKEY}")

    SEQ_P2P_KEY="${OUTPUT_DIR}/seq-p2p.hex"
    FN_P2P_KEY="${OUTPUT_DIR}/fn-p2p.hex"
    generate_key_file "${SEQ_P2P_KEY}"
    generate_key_file "${FN_P2P_KEY}"

    SEQ_P2P_PRIVKEY=$(cat "${SEQ_P2P_KEY}")
    FN_P2P_PRIVKEY=$(cat "${FN_P2P_KEY}")
    SEQ_P2P_PUBKEY=$(derive_enode_pubkey "${SEQ_P2P_PRIVKEY}")
    FN_P2P_PUBKEY=$(derive_enode_pubkey "${FN_P2P_PRIVKEY}")

    JWT_FILE="${OUTPUT_DIR}/jwt.hex"
    generate_key_file "${JWT_FILE}"

    SEQ_ROOT_KEY="${OUTPUT_DIR}/sequencer.key"
    if [ ! -f "${SEQ_ROOT_KEY}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genxpriv "${SEQ_ROOT_KEY}"
        echo "generated ${SEQ_ROOT_KEY}"
    fi

    OPERATOR_KEY="${OUTPUT_DIR}/operator.key"
    if [ ! -f "${OPERATOR_KEY}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genxpriv "${OPERATOR_KEY}"
        echo "generated ${OPERATOR_KEY}"
    fi
    OPERATOR_PK=$("${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genoppubkey -f "${OPERATOR_KEY}")

    SEQ_PK=$("${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genseqpubkey -f "${SEQ_ROOT_KEY}")

    L1_ANCHOR="${OUTPUT_DIR}/l1-anchor.json"
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

    EE_PARAMS="${OUTPUT_DIR}/ee-params.json"
    if [ ! -f "${EE_PARAMS}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
            gen-ee-params \
            -o "${EE_PARAMS}" \
            ${ALPEN_CHAIN_CONFIG:+--alpen-chain-config "$ALPEN_CHAIN_CONFIG"}
        echo "generated ${EE_PARAMS}"
    fi

    OL_PARAMS="${OUTPUT_DIR}/ol-params.json"
    if [ ! -f "${OL_PARAMS}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
            gen-ol-params \
            -o "${OL_PARAMS}" \
            -g "${GENESIS_L1_HEIGHT}" \
            --l1-anchor-file "${L1_ANCHOR}" \
            --ee-params "${EE_PARAMS}" \
            ${ALPEN_PREDICATE:+--alpen-predicate "$ALPEN_PREDICATE"} \
            ${ALPEN_CHAIN_CONFIG:+--alpen-chain-config "$ALPEN_CHAIN_CONFIG"}
        echo "generated ${OL_PARAMS}"
    fi

    ASM_PARAMS="${OUTPUT_DIR}/asm-params.json"
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
            ${CHECKPOINT_PREDICATE:+--checkpoint-predicate "$CHECKPOINT_PREDICATE"}
        echo "generated ${ASM_PARAMS}"
    fi

    ENV_FILE="${ENV_FILE:-${SCRIPT_DIR}/.env.alpen}"
    BRIDGE_RUNTIME_ENV="$(bridge_runtime_env "${OL_PARAMS}")"

    cat > "${ENV_FILE}" <<EOF
# Generated by init-network.sh -- do not edit.

BITCOIN_NETWORK=${BITCOIN_NETWORK}

SEQUENCER_PRIVATE_KEY=${SCHNORR_PRIVKEY}
SEQUENCER_PUBKEY=${SCHNORR_PUBKEY}

SEQ_P2P_PUBKEY=${SEQ_P2P_PUBKEY}
FN_P2P_PUBKEY=${FN_P2P_PUBKEY}

CHAIN_SPEC=${CHAIN_SPEC:-dev}
EE_PARAMS_PATH=/app/configs/generated/ee-params.json

OL_BLOCK_TIME_MS=${OL_BLOCK_TIME_MS:-5000}
ALPEN_EE_BLOCK_TIME_MS=${ALPEN_EE_BLOCK_TIME_MS:-5000}

EE_DA_MAGIC_BYTES=${EE_DA_MAGIC_BYTES:-ALPN}
L1_REORG_SAFE_DEPTH=${L1_REORG_SAFE_DEPTH:-4}
GENESIS_L1_HEIGHT=${GENESIS_L1_HEIGHT:-0}
BATCH_SEALING_BLOCK_COUNT=${BATCH_SEALING_BLOCK_COUNT:-5}
${BRIDGE_RUNTIME_ENV}

BITCOIND_RPC_USER=${BITCOIND_RPC_USER:-rpcuser}
BITCOIND_RPC_PASSWORD=${BITCOIND_RPC_PASSWORD:-rpcpassword}
BITCOIND_RPC_PORT=${BITCOIND_RPC_PORT:-${DEFAULT_RPC_PORT}}

STRATA_RPC_PORT=${STRATA_RPC_PORT:-8432}

SEQ_HTTP_PORT=${SEQ_HTTP_PORT:-8545}
SEQ_WS_PORT=${SEQ_WS_PORT:-8546}
SEQ_P2P_PORT=${SEQ_P2P_PORT:-30303}

FN_HTTP_PORT=${FN_HTTP_PORT:-9545}
FN_WS_PORT=${FN_WS_PORT:-9546}
FN_P2P_PORT=${FN_P2P_PORT:-31303}

RUST_LOG=${RUST_LOG:-info}
EOF

    echo "wrote ${ENV_FILE}"
    echo "network: ${BITCOIN_NETWORK}"
    echo "sequencer pubkey: ${SCHNORR_PUBKEY}"

elif [ "${MODE}" = "fullnode" ]; then
    echo "mode: fullnode"

    for f in ee-params.json ol-params.json asm-params.json; do
        if [ ! -f "${PARAMS_DIR}/${f}" ]; then
            echo "error: missing ${f} in ${PARAMS_DIR}" >&2
            exit 1
        fi
    done

    if [ "$(realpath "${PARAMS_DIR}")" != "$(realpath "${OUTPUT_DIR}")" ]; then
        for f in ee-params.json ol-params.json asm-params.json; do
            cp "${PARAMS_DIR}/${f}" "${OUTPUT_DIR}/${f}"
        done
        echo "copied params from ${PARAMS_DIR}"
    fi

    # The sequencer pubkey lives in the ASM checkpoint subprotocol's
    # `sequencer_predicate`, serialized as "Bip340Schnorr:<hex>" (or
    # "AlwaysAccept" when block signatures are unchecked).
    SEQUENCER_PUBKEY=$("${PYTHON}" -c "
import json, sys
params = json.load(open('${OUTPUT_DIR}/asm-params.json'))
checkpoint = None
for sub in params['subprotocols']:
    if isinstance(sub, dict) and 'Checkpoint' in sub:
        checkpoint = sub['Checkpoint']
        break
if checkpoint is None:
    sys.stderr.write('error: asm-params missing Checkpoint subprotocol\n')
    sys.exit(1)
pred = checkpoint['sequencer_predicate']
if isinstance(pred, str) and pred.startswith('Bip340Schnorr:'):
    sys.stdout.write(pred.split(':', 1)[1])
else:
    sys.stderr.write('warning: sequencer_predicate is not a schnorr key, no sequencer pubkey\n')
    sys.stdout.write('')
")

    if [ -z "${SEQUENCER_PUBKEY}" ]; then
        echo "error: could not extract sequencer pubkey from asm-params.json" >&2
        exit 1
    fi

    FN_P2P_KEY="${OUTPUT_DIR}/fn-p2p.hex"
    generate_key_file "${FN_P2P_KEY}"
    FN_P2P_PRIVKEY=$(cat "${FN_P2P_KEY}")
    FN_P2P_PUBKEY=$(derive_enode_pubkey "${FN_P2P_PRIVKEY}")

    ENV_FILE="${SCRIPT_DIR}/.env.alpen-fullnode"
    BRIDGE_RUNTIME_ENV="$(bridge_runtime_env "${OUTPUT_DIR}/ol-params.json")"

    cat > "${ENV_FILE}" <<EOF
# Generated by init-network.sh -- do not edit.

BITCOIN_NETWORK=${BITCOIN_NETWORK}

SEQUENCER_PUBKEY=${SEQUENCER_PUBKEY}

FN_P2P_PUBKEY=${FN_P2P_PUBKEY}

CHAIN_SPEC=${CHAIN_SPEC:-dev}
EE_PARAMS_PATH=/app/configs/generated/ee-params.json
${BRIDGE_RUNTIME_ENV}

FN_HTTP_PORT=${FN_HTTP_PORT:-9545}
FN_WS_PORT=${FN_WS_PORT:-9546}
FN_P2P_PORT=${FN_P2P_PORT:-31303}

RUST_LOG=${RUST_LOG:-info}
EOF

    echo "wrote ${ENV_FILE}"
    echo "network: ${BITCOIN_NETWORK}"
    echo "sequencer pubkey: ${SEQUENCER_PUBKEY}"
fi

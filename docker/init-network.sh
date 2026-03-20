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
# When BITCOIN_RPC_URL is set, the script fetches the real GenesisL1View from
# the Bitcoin node via `datatool genl1view`. Without it, a network-specific
# placeholder is used (suitable for regtest where the strata entrypoint patches
# params at runtime).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BITCOIN_NETWORK="${BITCOIN_NETWORK:-regtest}"
GENESIS_L1_HEIGHT="${GENESIS_L1_HEIGHT:-0}"
BITCOIN_RPC_URL="${BITCOIN_RPC_URL:-}"
BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-}"
BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-}"

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
            echo "  BITCOIN_RPC_URL       Bitcoin RPC URL (enables fetching real L1 view)"
            echo "  BITCOIN_RPC_USER      Bitcoin RPC username"
            echo "  BITCOIN_RPC_PASSWORD  Bitcoin RPC password"
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
    OPERATOR_XPRIV=$(cat "${OPERATOR_KEY}")

    SEQ_XPUB=$("${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" genseqpubkey -f "${SEQ_ROOT_KEY}")

    GENESIS_L1_VIEW="${OUTPUT_DIR}/genesis-l1-view.json"
    if [ ! -f "${GENESIS_L1_VIEW}" ]; then
        if [ -n "${BITCOIN_RPC_URL}" ] && [ -n "${BITCOIN_RPC_USER}" ] && [ -n "${BITCOIN_RPC_PASSWORD}" ]; then
            # Fetch real L1 view from Bitcoin node — produces correct values for
            # all fields (next_target, epoch_start_timestamp, last_11_timestamps).
            echo "fetching genesis L1 view from ${BITCOIN_RPC_URL} at height ${GENESIS_L1_HEIGHT}..."
            "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
                --bitcoin-rpc-url "${BITCOIN_RPC_URL}" \
                --bitcoin-rpc-user "${BITCOIN_RPC_USER}" \
                --bitcoin-rpc-password "${BITCOIN_RPC_PASSWORD}" \
                genl1view \
                -g "${GENESIS_L1_HEIGHT}" \
                -o "${GENESIS_L1_VIEW}"
            echo "generated ${GENESIS_L1_VIEW} (from Bitcoin RPC)"
        else
            # No RPC available — write a placeholder L1 view using network-specific
            # genesis block values.  On regtest the strata entrypoint patches
            # height + blkid at runtime; on signet this will be incomplete and you
            # should provide BITCOIN_RPC_* vars instead.
            if [ "${BITCOIN_NETWORK}" != "regtest" ] && [ "${GENESIS_L1_HEIGHT}" != "0" ]; then
                echo "warning: generating placeholder L1 view for ${BITCOIN_NETWORK} at height ${GENESIS_L1_HEIGHT}" >&2
                echo "         without Bitcoin RPC, next_target / timestamps will be WRONG." >&2
                echo "         Set BITCOIN_RPC_URL, BITCOIN_RPC_USER, BITCOIN_RPC_PASSWORD for correct values." >&2
            fi
            cat > "${GENESIS_L1_VIEW}" <<GEOF
{
  "blk": {
    "height": ${GENESIS_L1_HEIGHT},
    "blkid": "${GENESIS_BLKID}"
  },
  "next_target": ${L1_NEXT_TARGET},
  "epoch_start_timestamp": ${L1_EPOCH_START_TIMESTAMP},
  "last_11_timestamps": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
}
GEOF
            echo "generated ${GENESIS_L1_VIEW} (placeholder)"
        fi
    fi

    ROLLUP_PARAMS="${OUTPUT_DIR}/rollup-params.json"
    if [ ! -f "${ROLLUP_PARAMS}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
            genparams \
            -o "${ROLLUP_PARAMS}" \
            -n ALPN \
            -s "${SEQ_XPUB}" \
            -b "${OPERATOR_XPRIV}" \
            -g "${GENESIS_L1_HEIGHT}" \
            --proof-timeout 30 \
            --genesis-l1-view-file "${GENESIS_L1_VIEW}"
        echo "generated ${ROLLUP_PARAMS}"
    fi

    OL_PARAMS="${OUTPUT_DIR}/ol-params.json"
    if [ ! -f "${OL_PARAMS}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
            gen-ol-params \
            -o "${OL_PARAMS}" \
            -g "${GENESIS_L1_HEIGHT}" \
            --genesis-l1-view-file "${GENESIS_L1_VIEW}"
        echo "generated ${OL_PARAMS}"
    fi

    ASM_PARAMS="${OUTPUT_DIR}/asm-params.json"
    if [ ! -f "${ASM_PARAMS}" ]; then
        "${DATATOOL_PATH}" -b "${BITCOIN_NETWORK}" \
            gen-asm-params \
            -o "${ASM_PARAMS}" \
            -n ALPN \
            -b "${OPERATOR_XPRIV}" \
            -g "${GENESIS_L1_HEIGHT}" \
            --genesis-l1-view-file "${GENESIS_L1_VIEW}" \
            --ol-params "${OL_PARAMS}"
        echo "generated ${ASM_PARAMS}"
    fi

    ENV_FILE="${SCRIPT_DIR}/.env.alpen"

    cat > "${ENV_FILE}" <<EOF
# Generated by init-network.sh -- do not edit.

BITCOIN_NETWORK=${BITCOIN_NETWORK}

SEQUENCER_PRIVATE_KEY=${SCHNORR_PRIVKEY}
SEQUENCER_PUBKEY=${SCHNORR_PUBKEY}

SEQ_P2P_PUBKEY=${SEQ_P2P_PUBKEY}
FN_P2P_PUBKEY=${FN_P2P_PUBKEY}

CHAIN_SPEC=${CHAIN_SPEC:-dev}

OL_BLOCK_TIME_MS=${OL_BLOCK_TIME_MS:-5000}
ALPEN_EE_BLOCK_TIME_MS=${ALPEN_EE_BLOCK_TIME_MS:-5000}

EE_DA_MAGIC_BYTES=${EE_DA_MAGIC_BYTES:-ALPN}
L1_REORG_SAFE_DEPTH=${L1_REORG_SAFE_DEPTH:-4}
GENESIS_L1_HEIGHT=${GENESIS_L1_HEIGHT:-0}
BATCH_SEALING_BLOCK_COUNT=${BATCH_SEALING_BLOCK_COUNT:-5}

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

    for f in rollup-params.json ol-params.json asm-params.json; do
        if [ ! -f "${PARAMS_DIR}/${f}" ]; then
            echo "error: missing ${f} in ${PARAMS_DIR}" >&2
            exit 1
        fi
    done

    if [ "$(realpath "${PARAMS_DIR}")" != "$(realpath "${OUTPUT_DIR}")" ]; then
        for f in rollup-params.json ol-params.json asm-params.json; do
            cp "${PARAMS_DIR}/${f}" "${OUTPUT_DIR}/${f}"
        done
        echo "copied params from ${PARAMS_DIR}"
    fi

    SEQUENCER_PUBKEY=$("${PYTHON}" -c "
import json, sys
params = json.load(open('${OUTPUT_DIR}/rollup-params.json'))
cr = params['cred_rule']
if isinstance(cr, dict) and 'schnorr_key' in cr:
    sys.stdout.write(cr['schnorr_key'])
elif cr == 'unchecked':
    sys.stderr.write('warning: cred_rule is unchecked, no sequencer pubkey in params\n')
    sys.stdout.write('')
else:
    sys.stderr.write('error: unexpected cred_rule format\n')
    sys.exit(1)
")

    if [ -z "${SEQUENCER_PUBKEY}" ]; then
        echo "error: could not extract sequencer pubkey from rollup-params.json" >&2
        exit 1
    fi

    FN_P2P_KEY="${OUTPUT_DIR}/fn-p2p.hex"
    generate_key_file "${FN_P2P_KEY}"
    FN_P2P_PRIVKEY=$(cat "${FN_P2P_KEY}")
    FN_P2P_PUBKEY=$(derive_enode_pubkey "${FN_P2P_PRIVKEY}")

    ENV_FILE="${SCRIPT_DIR}/.env.alpen-fullnode"

    cat > "${ENV_FILE}" <<EOF
# Generated by init-network.sh -- do not edit.

BITCOIN_NETWORK=${BITCOIN_NETWORK}

SEQUENCER_PUBKEY=${SEQUENCER_PUBKEY}

FN_P2P_PUBKEY=${FN_P2P_PUBKEY}

CHAIN_SPEC=${CHAIN_SPEC:-dev}

FN_HTTP_PORT=${FN_HTTP_PORT:-9545}
FN_WS_PORT=${FN_WS_PORT:-9546}
FN_P2P_PORT=${FN_P2P_PORT:-31303}

RUST_LOG=${RUST_LOG:-info}
EOF

    echo "wrote ${ENV_FILE}"
    echo "network: ${BITCOIN_NETWORK}"
    echo "sequencer pubkey: ${SEQUENCER_PUBKEY}"
fi

#!/usr/bin/env bash
#
# init-alpen-client-keys.sh
#
# Generates all cryptographic material needed for the alpen-client Docker
# compose setup (1 sequencer + 2 fullnodes):
#
#   - 3 P2P secret keys  (secp256k1 private keys for RLPx)
#   - Computed enode URLs (uncompressed public keys derived from P2P keys)
#   - Sequencer Schnorr keypair (for gossip message signing/validation)
#   - JWT secrets (for Engine API auth between OL and EE)
#   - .env.alpen-client with all computed values
#
# Prerequisites:
#   pip install coincurve
#
# Usage:
#   cd vertex-core/docker
#   ./init-alpen-client-keys.sh
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/configs/alpen-client"

echo "==> Checking prerequisites..."

# Find a Python interpreter that has coincurve installed.
PYTHON=""
for candidate in python3 python3.12 python3.11 python3.10 python; do
    if command -v "${candidate}" &>/dev/null && "${candidate}" -c "import coincurve" 2>/dev/null; then
        PYTHON="${candidate}"
        break
    fi
done

if [ -z "${PYTHON}" ]; then
    echo "ERROR: No Python interpreter with 'coincurve' found."
    echo "Install it with: pip install coincurve"
    exit 1
fi

echo "    Using Python: $(command -v "${PYTHON}") ($(${PYTHON} --version 2>&1))"

echo "==> Creating output directory: ${OUTPUT_DIR}"
mkdir -p "${OUTPUT_DIR}"

# ---------------------------------------------------------------------------
# Helper: generate 32 random bytes as hex
# ---------------------------------------------------------------------------
generate_secret_key() {
    od -An -tx1 -N32 /dev/urandom | tr -d ' \n'
}

# ---------------------------------------------------------------------------
# Helper: derive uncompressed secp256k1 public key (sans 04 prefix) from
# a 32-byte hex private key.  Returns 128 hex chars (64 bytes).
# ---------------------------------------------------------------------------
derive_pubkey_uncompressed() {
    local privkey_hex="$1"
    "${PYTHON}" -c "
import coincurve
import sys

privkey_bytes = bytes.fromhex('${privkey_hex}')
pk = coincurve.PublicKey.from_secret(privkey_bytes)
# format=False gives 65-byte uncompressed (04 || x || y)
uncompressed = pk.format(compressed=False)
# Strip the 04 prefix → 64 bytes → 128 hex chars
sys.stdout.write(uncompressed[1:].hex())
"
}

# ---------------------------------------------------------------------------
# Helper: derive x-only (Schnorr) public key from a 32-byte hex private key.
# Returns 64 hex chars (32 bytes) — the x-coordinate of the public key.
# ---------------------------------------------------------------------------
derive_schnorr_pubkey() {
    local privkey_hex="$1"
    "${PYTHON}" -c "
import coincurve
import sys

privkey_bytes = bytes.fromhex('${privkey_hex}')
pk = coincurve.PublicKey.from_secret(privkey_bytes)
# Compressed is 33 bytes: prefix || x-coordinate
compressed = pk.format(compressed=True)
# x-only = drop the 1-byte prefix
sys.stdout.write(compressed[1:].hex())
"
}

# ---------------------------------------------------------------------------
# Generate P2P secret keys
# ---------------------------------------------------------------------------
echo "==> Generating P2P secret keys..."

P2P_SEQ_KEY="${OUTPUT_DIR}/p2p-seq.hex"
P2P_FN1_KEY="${OUTPUT_DIR}/p2p-fn1.hex"
P2P_FN2_KEY="${OUTPUT_DIR}/p2p-fn2.hex"

for keyfile in "${P2P_SEQ_KEY}" "${P2P_FN1_KEY}" "${P2P_FN2_KEY}"; do
    if [ -f "${keyfile}" ]; then
        echo "    ${keyfile} already exists, skipping."
    else
        generate_secret_key > "${keyfile}"
        echo "    Created ${keyfile}"
    fi
done

# Read back the private keys
SEQ_P2P_PRIVKEY=$(cat "${P2P_SEQ_KEY}")
FN1_P2P_PRIVKEY=$(cat "${P2P_FN1_KEY}")
FN2_P2P_PRIVKEY=$(cat "${P2P_FN2_KEY}")

# ---------------------------------------------------------------------------
# Derive uncompressed public keys → enode URLs
# ---------------------------------------------------------------------------
echo "==> Deriving public keys and enode URLs..."

SEQ_PUBKEY=$(derive_pubkey_uncompressed "${SEQ_P2P_PRIVKEY}")
FN1_PUBKEY=$(derive_pubkey_uncompressed "${FN1_P2P_PRIVKEY}")
FN2_PUBKEY=$(derive_pubkey_uncompressed "${FN2_P2P_PRIVKEY}")

# Enode URLs use Docker service hostnames on port 30303
ENODE_SEQ="enode://${SEQ_PUBKEY}@alpen-client-seq:30303"
ENODE_FN1="enode://${FN1_PUBKEY}@alpen-client-fn1:30303"
ENODE_FN2="enode://${FN2_PUBKEY}@alpen-client-fn2:30303"

echo "    Sequencer enode: ${ENODE_SEQ}"
echo "    Fullnode1 enode: ${ENODE_FN1}"
echo "    Fullnode2 enode: ${ENODE_FN2}"

# ---------------------------------------------------------------------------
# Generate Sequencer Schnorr keypair (for gossip signing)
# ---------------------------------------------------------------------------
echo "==> Generating sequencer Schnorr keypair..."

SEQ_SCHNORR_KEY="${OUTPUT_DIR}/sequencer-schnorr.hex"

if [ -f "${SEQ_SCHNORR_KEY}" ]; then
    echo "    ${SEQ_SCHNORR_KEY} already exists, skipping."
else
    generate_secret_key > "${SEQ_SCHNORR_KEY}"
    echo "    Created ${SEQ_SCHNORR_KEY}"
fi

SEQ_SCHNORR_PRIVKEY=$(cat "${SEQ_SCHNORR_KEY}")
SEQ_SCHNORR_PUBKEY=$(derive_schnorr_pubkey "${SEQ_SCHNORR_PRIVKEY}")

echo "    Sequencer Schnorr pubkey: ${SEQ_SCHNORR_PUBKEY}"

# ---------------------------------------------------------------------------
# Generate JWT secrets (for Engine API auth)
# ---------------------------------------------------------------------------
echo "==> Generating JWT secrets..."

JWT_SEQ="${OUTPUT_DIR}/jwt-seq.hex"
JWT_FN1="${OUTPUT_DIR}/jwt-fn1.hex"
JWT_FN2="${OUTPUT_DIR}/jwt-fn2.hex"

for jwtfile in "${JWT_SEQ}" "${JWT_FN1}" "${JWT_FN2}"; do
    if [ -f "${jwtfile}" ]; then
        echo "    ${jwtfile} already exists, skipping."
    else
        generate_secret_key > "${jwtfile}"
        echo "    Created ${jwtfile}"
    fi
done

# ---------------------------------------------------------------------------
# Build trusted-peers lists (each node trusts the other two)
# ---------------------------------------------------------------------------
TRUSTED_PEERS_SEQ="${ENODE_FN1},${ENODE_FN2}"
TRUSTED_PEERS_FN1="${ENODE_SEQ},${ENODE_FN2}"
TRUSTED_PEERS_FN2="${ENODE_SEQ},${ENODE_FN1}"

# ---------------------------------------------------------------------------
# Write .env.alpen-client
# ---------------------------------------------------------------------------
ENV_FILE="${SCRIPT_DIR}/.env.alpen-client"

echo "==> Writing ${ENV_FILE}..."

cat > "${ENV_FILE}" <<EOF
# ============================================================================
# Generated by init-alpen-client-keys.sh — do not edit manually.
# Re-run the script to regenerate (existing keys are preserved).
# ============================================================================

# --- Sequencer Schnorr keys (gossip signing) --------------------------------
SEQUENCER_PRIVATE_KEY=${SEQ_SCHNORR_PRIVKEY}
SEQUENCER_PUBKEY=${SEQ_SCHNORR_PUBKEY}

# --- Trusted peer lists (enode URLs) ----------------------------------------
TRUSTED_PEERS_SEQ=${TRUSTED_PEERS_SEQ}
TRUSTED_PEERS_FN1=${TRUSTED_PEERS_FN1}
TRUSTED_PEERS_FN2=${TRUSTED_PEERS_FN2}

# --- Chain spec (dev | devnet | testnet) ------------------------------------
CHAIN_SPEC=testnet

# --- Bitcoin RPC -------------------------------------------------------------
BITCOIND_RPC_USER=rpcuser
BITCOIND_RPC_PASSWORD=rpcpassword

# --- Logging ----------------------------------------------------------------
RUST_LOG=info
EOF

echo ""
echo "=== Done ==="
echo ""
echo "Generated files:"
echo "  P2P keys:       ${OUTPUT_DIR}/p2p-{seq,fn1,fn2}.hex"
echo "  Schnorr key:    ${OUTPUT_DIR}/sequencer-schnorr.hex"
echo "  JWT secrets:    ${OUTPUT_DIR}/jwt-{seq,fn1,fn2}.hex"
echo "  Environment:    ${ENV_FILE}"
echo ""
echo "Next steps:"
echo "  cd ${SCRIPT_DIR}"
echo "  docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml build"
echo "  docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml up"

#!/bin/sh
# Entrypoint script

set -eu
# Fail fast on errors and unset variables

umask 027
# Restrict default permissions for newly created files


CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
PARAM_PATH=${PARAM_PATH:-/config/params.json}

# Validate required files exist
[ -f "${CONFIG_PATH}" ] || {
  echo "Error: missing config '${CONFIG_PATH}'" >&2
  exit 1
}

[ -f "${PARAM_PATH}" ] || {
  echo "Error: missing params '${PARAM_PATH}'" >&2
  exit 1
}

exec strata \
  --config "${CONFIG_PATH}" \
  --rollup-params "${PARAM_PATH}" \
  "$@"

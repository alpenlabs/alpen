#!/bin/sh
set -eu

umask 027

CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
PARAM_PATH=${PARAM_PATH:-/config/params.json}

if [ ! -f "${CONFIG_PATH}" ]; then
    echo "Error: missing config file '${CONFIG_PATH}'." >&2
    exit 1
fi

if [ -n "${PARAM_PATH}" ] && [ ! -f "${PARAM_PATH}" ]; then
    echo "Error: missing params file '${PARAM_PATH}'." >&2
    exit 1
fi

set -- --config "${CONFIG_PATH}" "$@"
if [ -n "${PARAM_PATH}" ]; then
    set -- "$@" --rollup-params "${PARAM_PATH}"
fi

exec strata "$@"

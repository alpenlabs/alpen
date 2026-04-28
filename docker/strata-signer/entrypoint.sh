#!/bin/sh
set -eu
umask 027

CONFIG_PATH=${CONFIG_PATH:-/app/configs/strata-signer/config.toml}

[ -f "${CONFIG_PATH}" ] || { echo "error: missing config '${CONFIG_PATH}'" >&2; exit 1; }

exec strata-signer -c "${CONFIG_PATH}" "$@"

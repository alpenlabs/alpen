#!/bin/sh
# Entrypoint script

set -eu
# Fail fast on errors and unset variables

umask 027
# Restrict default permissions for newly created files


if [ "${1-}" = "help" ] || [ "${1-}" = "--help" ] || [ "${1-}" = "-h" ]; then
    exec alpen-client --help
fi

exec alpen-client "$@"

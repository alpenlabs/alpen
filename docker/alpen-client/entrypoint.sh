#!/bin/sh

# Fail fast on errors and unset variables
set -eu

# Restrict default permissions for newly created files
umask 027


if [ "${1-}" = "help" ] || [ "${1-}" = "--help" ] || [ "${1-}" = "-h" ]; then
    exec alpen-client --help
fi

exec alpen-client "$@"

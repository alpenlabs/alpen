#!/bin/sh
set -eu

umask 027

if [ "${1-}" = "help" ] || [ "${1-}" = "--help" ] || [ "${1-}" = "-h" ]; then
    exec alpen-client --help
fi

exec alpen-client "$@"

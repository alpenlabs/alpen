#!/bin/bash

set -e -u

verbose=
if [ "$1" = "-v" ]; then
	verbose=1
	shift 1
fi

cd "$(dirname "$(realpath "$0")")/_dd"

# shellcheck disable=2012
last_dd=$(ls -t . | head -n 1)

[ "$verbose" ] && echo "last datadir:" "$last_dd"
cd "$last_dd"
[ "$verbose" ] && echo "envs:" "$(ls)"
cd "$1"
[ "$verbose" ] && echo "services:" "$(ls)"
cd "$2"

shift 2
exec "$@"

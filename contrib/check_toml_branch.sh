#!/usr/bin/env bash
#
# Looks for Cargo.toml files that have `branch = ` in them. This should always
# work if the toml file passes the toml style check.  This can be overridden by
# adding `~ignorebranch` on the same line in a comment, if we have some really
# good reason to ignore it.
#
# This script was created because of some nightmares that happened when we let
# this get into main.

found=0

for p in $(find . -name 'Cargo.toml'); do
    lines=$(grep -E 'branch = ' $p | grep -v "~ignorebranch")
    if [ ! -z "$lines" ]; then
        echo "found branch selector on dependency in $p"
        found=1
    fi
done

exit $found


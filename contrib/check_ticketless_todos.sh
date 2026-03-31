#!/usr/bin/env bash

# Check that newly added TODO/FIXME comments include a ticket reference.
#
# Valid:   TODO(STR-2105), FIXME(#123), TODO(PROJ-42)
# Invalid: TODO, TODO:, FIXME without a ticket
#
# Usage:
#   ./contrib/check_ticketless_todos.sh [base_ref]
#
# base_ref defaults to "main". Only added lines in the diff are checked.

set -euo pipefail

base_ref="${1:-main}"

# Get only added lines (starting with +) from the diff of tracked file types.
# Exclude the +++ diff header lines.
added_lines=$(git diff "$base_ref"...HEAD -- '*.rs' '*.py' '*.sh' '*.toml' '*.yml' '*.yaml' \
    | grep -E '^\+[^+]' || true)

if [ -z "$added_lines" ]; then
    exit 0
fi

# Match TODO or FIXME that are NOT immediately followed by '(' (which would contain a ticket ref).
# This catches: TODO, TODO:, TODO should, FIXME:, FIXME this, etc.
# But allows: TODO(STR-123), FIXME(#456), TODO(PROJ-78)
ticketless=$(echo "$added_lines" \
    | grep -E '\b(TODO|FIXME)\b' \
    | grep -vE '\b(TODO|FIXME)\([A-Za-z]+-[0-9]+\)' \
    | grep -vE '\b(TODO|FIXME)\(#[0-9]+\)' \
    || true)

if [ -n "$ticketless" ]; then
    echo "ERROR: Found TODO/FIXME without a ticket reference."
    echo "       Use the format TODO(PROJ-123) or FIXME(#456) instead."
    echo ""
    echo "$ticketless"
    exit 1
fi

exit 0

#!/usr/bin/env bash

# Check for hand-written SSZ `Encode`/`Decode` impls (STR-3001).
#
# SSZ requires a specific buffer layout: the fixed part of a container comes first,
# followed by the variable part, with offsets for variable-length fields. Manual
# impls modeled on Borsh/strata-codec (append each field in order) can silently
# produce non-spec-compliant bytes. To keep SSZ encodings correct by construction,
# SSZ types should be generated from `.ssz` files (ssz-gen), use the
# `#[derive(Encode, Decode)]` macros, or delegate byte handling to a derived mirror
# type. See https://github.com/ethereum/consensus-specs/blob/master/ssz/simple-serialize.md
#
# This guard flags the SSZ trait *method* signatures (which appear in source only for
# manual impls; derive/codegen emit them inside macro expansion). A genuinely
# necessary manual impl — e.g. a decode that delegates to a derived mirror and only
# adds validation — must justify itself with an inline marker comment:
#
#   // ssz-manual-ok: <reason>
#
# placed on the `impl` line or the line immediately above it. The marker covers every
# SSZ method in that impl block.
#
# Usage:
#   ./contrib/check_manual_ssz_impls.sh [dir ...]
#
# Directories default to "crates" and "bin". Exits 1 if any unmarked manual SSZ impl
# is found, 0 otherwise.

set -euo pipefail

dirs=("$@")
if [ ${#dirs[@]} -eq 0 ]; then
    dirs=(crates bin)
fi

files=$(find "${dirs[@]}" -name '*.rs' -type f 2>/dev/null || true)

if [ -z "$files" ]; then
    exit 0
fi

# For each .rs file, an SSZ trait method (ssz_append, ssz_bytes_len, from_ssz_bytes,
# is_ssz_fixed_len, ssz_fixed_len) is a violation unless its enclosing `impl` block —
# or the method line itself — carries the `ssz-manual-ok` marker.
# shellcheck disable=SC2016  # `$0`/fields below are awk syntax, not shell expansions.
violations=$(echo "$files" | xargs awk '
    {
        has_marker = ($0 ~ /ssz-manual-ok/)
        is_impl    = ($0 ~ /^[[:space:]]*impl[[:space:]]/)
        is_ssz_fn  = ($0 ~ /fn[[:space:]]+(ssz_append|ssz_bytes_len|from_ssz_bytes|is_ssz_fixed_len|ssz_fixed_len)[[:space:]]*\(/)

        if (is_impl) {
            allow = (has_marker || prev_marker)
        }
        if (is_ssz_fn && !(allow || has_marker || prev_marker)) {
            sub(/^[[:space:]]+/, "", $0)
            print FILENAME ":" FNR ": " $0
            found = 1
        }
        prev_marker = has_marker
    }
    END { if (found) exit 1 }
' 2>&1) && status=0 || status=$?

if [ "${status:-0}" -ne 0 ]; then
    echo "ERROR: Found hand-written SSZ Encode/Decode impl(s) without justification."
    echo ""
    echo "$violations"
    echo ""
    echo "Make SSZ encodings correct by construction instead of hand-rolling the layout:"
    echo "  - generate the type from a .ssz file (ssz-gen), or"
    echo "  - use the #[derive(Encode, Decode)] macros, or"
    echo "  - delegate byte handling to a derived mirror type and add validation only."
    echo ""
    echo "If a manual impl is genuinely required, justify it with an inline comment"
    echo "on (or directly above) the impl line:"
    echo "  // ssz-manual-ok: <reason>"
    exit 1
fi

exit 0

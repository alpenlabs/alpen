#!/usr/bin/env bash

# Check that newly added task-marker comments include a ticket reference.
#
# Valid markers include a Jira key or GitHub issue number immediately after the marker word.
# Invalid markers omit that parenthesized reference.
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

# Match the task markers that are not followed by an accepted parenthesized reference.
todo_word='TO''DO'
fixme_word='FIX''ME'
marker_regex="\\b(${todo_word}|${fixme_word})\\b"
jira_marker_regex="\\b(${todo_word}|${fixme_word})\\([A-Za-z]+-[0-9]+\\)"
github_marker_regex="\\b(${todo_word}|${fixme_word})\\(#[0-9]+\\)"

ticketless=$(echo "$added_lines" \
    | grep -E "$marker_regex" \
    | grep -vE "$jira_marker_regex" \
    | grep -vE "$github_marker_regex" \
    || true)

if [ -n "$ticketless" ]; then
    echo "ERROR: Found a task marker without a ticket reference."
    echo "       Put a Jira key or GitHub issue number in parentheses immediately after the marker."
    echo ""
    echo "$ticketless"
    exit 1
fi

exit 0

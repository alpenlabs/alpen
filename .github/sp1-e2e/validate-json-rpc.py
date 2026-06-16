#!/usr/bin/env python3
"""Validates a JSON-RPC response and prints a failure reason."""

from __future__ import annotations

import json
import sys
from typing import Any


def compact_json(value: Any) -> str:
    """Returns a compact JSON string for log output."""

    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def failure_reason(label: str, raw_response: str) -> str | None:
    """Returns a failure reason when a response is not a successful JSON-RPC result."""

    try:
        response = json.loads(raw_response)
    except json.JSONDecodeError as err:
        return f"{label} RPC returned invalid JSON: {err}"

    if not isinstance(response, dict):
        return f"{label} RPC returned non-object JSON: {compact_json(response)}"

    if "error" in response:
        return f"{label} RPC returned error: {compact_json(response['error'])}"

    if "result" not in response:
        return f"{label} RPC response missing result: {compact_json(response)}"

    return None


def main() -> int:
    """Validates stdin as a JSON-RPC response."""

    if len(sys.argv) != 2:
        print("usage: validate-json-rpc.py <label>", file=sys.stderr)
        return 2

    reason = failure_reason(sys.argv[1], sys.stdin.read())
    if reason is None:
        return 0

    print(reason)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

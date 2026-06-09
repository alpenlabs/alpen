#!/usr/bin/env python3
"""Groups WARN and ERROR docker compose logs by service prefix."""

from __future__ import annotations

import re
import sys
from collections import OrderedDict
from pathlib import Path


ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
LEVEL_RE = re.compile(r"\b(WARN|ERROR)\b")


def strip_ansi(line: str) -> str:
    return ANSI_RE.sub("", line)


def split_service(line: str) -> tuple[str, str]:
    service, sep, message = line.partition(" | ")
    if not sep:
        return "(unknown)", line.rstrip()
    return service.rstrip(), message.rstrip()


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: summarize-warn-error-logs.py <log-file>", file=sys.stderr)
        return 2

    log_path = Path(sys.argv[1])
    if not log_path.is_file():
        print("No logs collected.")
        return 0

    grouped: OrderedDict[str, dict[str, object]] = OrderedDict()

    with log_path.open("r", encoding="utf-8", errors="replace") as logs:
        for raw_line in logs:
            line = strip_ansi(raw_line.rstrip("\n"))
            match = LEVEL_RE.search(line)
            if not match:
                continue

            service, message = split_service(line)
            service_summary = grouped.setdefault(
                service,
                {"WARN": 0, "ERROR": 0, "lines": []},
            )
            level = match.group(1)
            service_summary[level] = int(service_summary[level]) + 1
            service_summary["lines"].append(f"  {level}: {message}")

    if not grouped:
        print("No WARN or ERROR logs found.")
        return 0

    print("WARN/ERROR counts by service")
    for service, service_summary in grouped.items():
        print(
            f"{service}: ERROR={service_summary['ERROR']}, WARN={service_summary['WARN']}"
        )

    print()
    print("WARN/ERROR lines by service")
    for service, service_summary in grouped.items():
        print()
        print(service)
        for line in service_summary["lines"]:
            print(line)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

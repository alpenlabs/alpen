#!/usr/bin/env python3
"""Detect anyhow::Error wrapped with #[from] or #[source] inside thiserror error types.

Wrapping anyhow::Error this way obscures the original error's type information,
making debugging and tracing harder. Use a specific error type instead.
"""

import argparse
import re
import sys
from pathlib import Path


def check_file(path: Path) -> list[tuple[int, list[str]]]:
    """Return list of (line_number, all_lines) for violations in the file."""
    try:
        text = path.read_text()
    except (OSError, UnicodeDecodeError):
        return []

    # Quick bail: file must use both thiserror and anyhow::Error
    if "thiserror" not in text or "anyhow::Error" not in text:
        return []

    lines = text.splitlines()
    violations: list[tuple[int, list[str]]] = []

    # We look for anyhow::Error on a line that is inside a thiserror-derived
    # enum/struct definition and preceded by a #[from] or #[source] attribute
    # (possibly on the same line or a nearby preceding line).

    in_thiserror_block = False
    brace_depth = 0
    recent_from_or_source = False
    from_source_line = 0

    for i, line in enumerate(lines, start=1):
        stripped = line.strip()

        # Detect start of a thiserror-derived type
        if re.search(r"#\[derive\(.*(?:Error|thiserror::Error)", stripped):
            in_thiserror_block = True
            brace_depth = 0
            continue

        if in_thiserror_block:
            brace_depth += line.count("{") - line.count("}")

            if brace_depth <= 0 and "{" not in line and "}" in line:
                in_thiserror_block = False
                recent_from_or_source = False
                continue

            # Track #[from] or #[source] attributes
            if re.search(r"#\[(from|source)\]", stripped):
                recent_from_or_source = True
                from_source_line = i

            # Check for anyhow::Error on a line with or shortly after #[from]/#[source]
            if "anyhow::Error" in stripped:
                # Same line has #[from]/#[source], or it was on a recent preceding line
                same_line = re.search(r"#\[(from|source)\]", stripped)
                if same_line or (recent_from_or_source and i - from_source_line <= 2):
                    violations.append((i, lines))

            # Reset the flag once we've passed the line with the field
            if (
                recent_from_or_source
                and i > from_source_line
                and stripped
                and not stripped.startswith("//")
                and not stripped.startswith("#[")
            ):
                recent_from_or_source = False

    return violations


def format_violation(
    path: Path, root: Path, lineno: int, lines: list[str], context: int
) -> str:
    """Format a single violation with optional context lines."""
    relpath = path.relative_to(root)
    if context == 0:
        return f"  {relpath}:{lineno}:{lines[lineno - 1]}"

    total = len(lines)
    start = max(0, lineno - 1 - context)
    end = min(total, lineno + context)
    # Width for line numbers in the gutter
    gutter = len(str(end))

    parts = [f"  {relpath}:{lineno}"]
    for idx in range(start, end):
        num = idx + 1
        marker = ">" if num == lineno else " "
        parts.append(f"  {marker} {num:{gutter}} | {lines[idx].rstrip()}")
    return "\n".join(parts)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Detect anyhow::Error wrapped with #[from]/#[source] in thiserror types."
    )
    parser.add_argument(
        "-C",
        "--context",
        type=int,
        default=2,
        metavar="N",
        help="show N lines of context around each violation",
    )
    args = parser.parse_args()

    crates_dir = Path(__file__).resolve().parent.parent / "crates"
    if not crates_dir.is_dir():
        print(f"error: {crates_dir} not found", file=sys.stderr)
        return 1

    all_violations: list[tuple[Path, int, list[str]]] = []

    for rs_file in sorted(crates_dir.rglob("*.rs")):
        for lineno, lines in check_file(rs_file):
            all_violations.append((rs_file, lineno, lines))

    if all_violations:
        print(
            "Found anyhow::Error wrapped with #[from]/#[source] in thiserror types:\n"
        )
        for path, lineno, lines in all_violations:
            print(
                format_violation(path, crates_dir.parent, lineno, lines, args.context)
            )
            if args.context:
                print()
        print(f"{len(all_violations)} violation(s) found.")
        print(
            "Use a specific error type instead of anyhow::Error with #[from]/#[source]."
        )
        return 1

    print("No anyhow::Error in thiserror violations found.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Validate the anyOS xfstests parity manifest."""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path


VALID_STATUSES = {
    "native",
    "adapted",
    "covered",
    "blocked",
    "unsupported",
    "not-applicable",
    "todo",
}

EXPECTED_HEADER = [
    "suite",
    "id",
    "upstream_path",
    "upstream_commit",
    "groups",
    "required_features",
    "status",
    "anyos_test",
    "reason",
    "notes",
]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--expected-total", type=int, default=2155)
    args = parser.parse_args(argv)

    with args.manifest.open(newline="") as f:
        reader = csv.DictReader(f)
        if reader.fieldnames != EXPECTED_HEADER:
            print(f"bad header: {reader.fieldnames}", file=sys.stderr)
            return 1
        rows = list(reader)

    errors: list[str] = []
    if len(rows) != args.expected_total:
        errors.append(f"expected {args.expected_total} rows, got {len(rows)}")

    seen: set[tuple[str, str]] = set()
    for idx, row in enumerate(rows, start=2):
        key = (row["suite"], row["id"])
        if key in seen:
            errors.append(f"line {idx}: duplicate {row['suite']}/{row['id']}")
        seen.add(key)
        if row["status"] not in VALID_STATUSES:
            errors.append(f"line {idx}: invalid status {row['status']!r}")
        expected_path = f"tests/{row['suite']}/{row['id']}"
        if row["upstream_path"] != expected_path:
            errors.append(
                f"line {idx}: upstream_path {row['upstream_path']!r} != {expected_path!r}"
            )
        if row["status"] in {"blocked", "unsupported", "not-applicable"} and not row[
            "reason"
        ]:
            errors.append(f"line {idx}: {row['status']} row needs reason")
        if row["status"] in {"native", "adapted", "covered"} and not row["anyos_test"]:
            errors.append(f"line {idx}: {row['status']} row needs anyos_test")

    if errors:
        for error in errors[:50]:
            print(error, file=sys.stderr)
        if len(errors) > 50:
            print(f"... and {len(errors) - 50} more errors", file=sys.stderr)
        return 1

    print(f"manifest ok: {len(rows)} rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

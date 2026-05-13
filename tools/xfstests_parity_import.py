#!/usr/bin/env python3
"""Generate the anyOS xfstests parity manifest.

The manifest is intentionally simple CSV so it can be reviewed in git and
post-processed by shell, Python, or future Rust harness code.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


NUMBERED_TEST = re.compile(r"^[0-9]{3}$")
BEGIN_RE = re.compile(r"^\s*_begin_fstest\s+(.+?)\s*$", re.MULTILINE)
REQUIRE_RE = re.compile(r"(?<![A-Za-z0-9_])(_require_[A-Za-z0-9_]+)")

NETWORK_OR_FOREIGN_SUITES = {
    "ceph": "network filesystem suite; no anyOS Ceph client target yet",
    "cifs": "network filesystem suite; no anyOS CIFS client target yet",
    "nfs": "network filesystem suite; no anyOS NFS client target yet",
    "ocfs2": "cluster filesystem suite; no anyOS cluster filesystem target",
    "udf": "UDF-specific suite; no anyOS UDF target",
}


@dataclass(order=True)
class TestEntry:
    suite: str
    test_id: str
    upstream_path: str
    upstream_commit: str
    groups: str
    required_features: str
    status: str
    anyos_test: str
    reason: str
    notes: str


def run_git(args: list[str], cwd: Path) -> str:
    try:
        return subprocess.check_output(["git", *args], cwd=cwd, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def parse_begin_groups(text: str) -> list[str]:
    match = BEGIN_RE.search(text)
    if not match:
        return []
    raw = match.group(1)
    groups: list[str] = []
    for token in raw.split():
        token = token.strip("\"'")
        if token and not token.startswith("$"):
            groups.append(token)
    return sorted(dict.fromkeys(groups))


def parse_requires(text: str) -> list[str]:
    requires = REQUIRE_RE.findall(text)
    return sorted(dict.fromkeys(requires))


def initial_status(suite: str) -> tuple[str, str]:
    if suite in NETWORK_OR_FOREIGN_SUITES:
        return "not-applicable", NETWORK_OR_FOREIGN_SUITES[suite]
    if suite == "overlay":
        return "blocked", "overlay/whiteout filesystem semantics are not available in anyOS yet"
    return "todo", "needs parity classification"


def iter_numbered_tests(xfstests_dir: Path, commit: str) -> list[TestEntry]:
    tests_dir = xfstests_dir / "tests"
    if not tests_dir.is_dir():
        raise SystemExit(f"xfstests tests directory not found: {tests_dir}")

    entries: list[TestEntry] = []
    for suite_dir in sorted(p for p in tests_dir.iterdir() if p.is_dir()):
        suite = suite_dir.name
        for test_file in sorted(p for p in suite_dir.iterdir() if p.is_file()):
            if not NUMBERED_TEST.match(test_file.name):
                continue
            text = test_file.read_text(errors="replace")
            status, reason = initial_status(suite)
            entries.append(
                TestEntry(
                    suite=suite,
                    test_id=test_file.name,
                    upstream_path=f"tests/{suite}/{test_file.name}",
                    upstream_commit=commit,
                    groups=";".join(parse_begin_groups(text)),
                    required_features=";".join(parse_requires(text)),
                    status=status,
                    anyos_test="",
                    reason=reason,
                    notes="",
                )
            )
    return entries


def write_manifest(entries: list[TestEntry], out_path: Path) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(
            [
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
        )
        for e in entries:
            writer.writerow(
                [
                    e.suite,
                    e.test_id,
                    e.upstream_path,
                    e.upstream_commit,
                    e.groups,
                    e.required_features,
                    e.status,
                    e.anyos_test,
                    e.reason,
                    e.notes,
                ]
            )


def write_summary(entries: list[TestEntry], out_path: Path) -> None:
    by_suite: dict[str, int] = {}
    by_status: dict[str, int] = {}
    by_group: dict[str, int] = {}
    for e in entries:
        by_suite[e.suite] = by_suite.get(e.suite, 0) + 1
        by_status[e.status] = by_status.get(e.status, 0) + 1
        for group in filter(None, e.groups.split(";")):
            by_group[group] = by_group.get(group, 0) + 1

    summary = {
        "total": len(entries),
        "by_suite": dict(sorted(by_suite.items())),
        "by_status": dict(sorted(by_status.items())),
        "by_group": dict(sorted(by_group.items())),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--xfstests-dir",
        required=True,
        type=Path,
        help="Path to a local kdave/xfstests checkout.",
    )
    parser.add_argument(
        "--out",
        default=Path("tests/xfstests-parity/manifest.csv"),
        type=Path,
        help="Output CSV path.",
    )
    parser.add_argument(
        "--summary",
        default=Path("tests/xfstests-parity/summary.json"),
        type=Path,
        help="Output JSON summary path.",
    )
    args = parser.parse_args(argv)

    xfstests_dir = args.xfstests_dir.resolve()
    commit = run_git(["rev-parse", "HEAD"], xfstests_dir)
    entries = iter_numbered_tests(xfstests_dir, commit)
    write_manifest(entries, args.out)
    write_summary(entries, args.summary)
    print(f"wrote {len(entries)} xfstests entries to {args.out}")
    print(f"wrote summary to {args.summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

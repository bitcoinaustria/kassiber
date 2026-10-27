"""Replay recorded sessions: `python -m tests.session_harness replay <dir>...`.

`<dir>` is a recording directory (`KASSIBER_TEST_RECORD_SESSIONS`) or one
session in it. `--candidate` takes the same forms as `KASSIBER_TEST_CLI`
(which is also its default); without either, the candidate is a fresh run of
this checkout's Python reference.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

from tests import cli_runner
from tests.session_harness.replay import replay, session_dirs


def explanation(report: dict) -> str:
    """The first diagnostic relaxation under which a different session is clean."""
    for label, count in report.get("diagnostics", {}).items():
        if count == 0:
            return label
    return "unexplained"


def _replay_one(path: str, candidate, keep: bool, diagnose: bool) -> dict:
    return replay(path, candidate, keep=keep, diagnose=diagnose).as_dict()


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tests.session_harness")
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("replay", help="replay sessions against a candidate and compare")
    run.add_argument("paths", nargs="+")
    run.add_argument("--candidate", help="executable path or JSON argv list, as KASSIBER_TEST_CLI")
    run.add_argument("--jobs", type=int, default=1)
    run.add_argument("--json", dest="json_out", help="write the full report here")
    run.add_argument("--keep", action="store_true", help="keep the replay directories")
    run.add_argument("--show", type=int, default=3, help="differences to print per session")
    run.add_argument(
        "--diagnose",
        action="store_true",
        help="also compare with clock readings and entropy relaxed, to see what differences are made of",
    )
    args = parser.parse_args(argv)

    value = args.candidate or os.environ.get(cli_runner.CLI_ENV)
    candidate = cli_runner.parse_candidate(value) if value else None
    sessions = [str(path) for path in session_dirs(args.paths)]
    if not sessions:
        parser.error("no recorded sessions found")
    if args.jobs > 1:
        with ProcessPoolExecutor(max_workers=args.jobs) as pool:
            reports = list(
                pool.map(
                    _replay_one,
                    sessions,
                    [candidate] * len(sessions),
                    [args.keep] * len(sessions),
                    [args.diagnose] * len(sessions),
                )
            )
    else:
        reports = [_replay_one(path, candidate, args.keep, args.diagnose) for path in sessions]
    counts = collections.Counter(report["status"] for report in reports)
    for report in reports:
        if report["status"] == "clean":
            continue
        print(f"{report['status']}: {report['session']} ({', '.join(report['tests'][:2])})")
        for note in report["notes"][:args.show]:
            print(f"  note: {note}")
        for difference in report["differences"][:args.show]:
            print("  " + json.dumps(difference, ensure_ascii=False)[:600])
    print(f"sessions: {len(reports)}; " + ", ".join(f"{status}: {count}" for status, count in sorted(counts.items())))
    if args.diagnose:
        explained = collections.Counter(explanation(report) for report in reports if report["status"] == "different")
        print("different sessions, by the first relaxation that clears them: " + ", ".join(
            f"{label}: {count}" for label, count in sorted(explained.items())
        ))
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(reports, indent=2) + "\n", encoding="utf-8")
    return 0 if set(counts) <= {"clean", "refused"} else 1


if __name__ == "__main__":
    sys.exit(main())

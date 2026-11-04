#!/usr/bin/env python3
"""Render and check the report-design prototype for the sample books.

    uv run --locked python crates/reports/scripts/report_design_prototype.py \\
        --out target/report-design --bin target/debug/examples/report_design

Needs the books from ``report_design_books.py`` under ``--out/books``. For each
book it runs the Rust renderer twice in separate processes and requires
identical bytes, requires the model handed to the renderers (``model.json``)
to equal Python's data for the same book (``python-model.json``) exactly,
then keeps one run's PDF, XLSX and CSV bundle and renders every PDF page to
PNG with ``pdftoppm``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path


def digest(root: Path) -> dict[str, str]:
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()}


def canonical(path: Path) -> str:
    return json.dumps(json.loads(path.read_text()), sort_keys=True, ensure_ascii=False)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--bin", required=True)
    args = parser.parse_args()
    out = Path(args.out).resolve()
    failures = []
    report = {}
    for book in ("austria", "generic"):
        database = out / "books" / book / "data" / "kassiber.sqlite3"
        runs = []
        for run in ("run-1", "run-2"):
            target = out / "runs" / book / run
            shutil.rmtree(target, ignore_errors=True)
            target.mkdir(parents=True)
            subprocess.run([args.bin, str(database), str(target)], check=True, capture_output=True, text=True)
            runs.append(digest(target))
        if runs[0] != runs[1]:
            failures.append(f"{book}: two runs differ")
        first = out / "runs" / book / "run-1"
        model_equal = canonical(first / "model.json") == canonical(out / "books" / book / "python-model.json")
        if not model_equal:
            failures.append(f"{book}: the Rust model differs from Python's")
        final = out / book
        shutil.rmtree(final, ignore_errors=True)
        shutil.copytree(first, final)
        subprocess.run(["pdftoppm", "-r", "110", "-png", str(final / "summary.pdf"), str(final / "page")], check=True)
        report[book] = {
            "deterministic": runs[0] == runs[1],
            "model_equals_python": model_equal,
            "files": runs[0],
            "pages": sorted(p.name for p in final.glob("page-*.png")),
        }
    shutil.rmtree(out / "runs", ignore_errors=True)
    (out / "prototype-check.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({book: {k: v for k, v in data.items() if k != "files"} for book, data in report.items()}, indent=2))
    for failure in failures:
        print("FAILED", failure, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())

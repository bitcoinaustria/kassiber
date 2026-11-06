#!/usr/bin/env python3
#!/usr/bin/env python3
"""Compare desktop daemon kinds between the Python and the Rust daemon.

    .venv/bin/python crates/daemon/scripts/desktop_read_diff.py \
        --rust-bin target/release/kassiber --out <disposable dir> \
        [--book <data root>] [--args-file args.json] [--kinds k1,k2 | --extra k1,k2] \
        [--python-src <exported source tree>]

Each side runs `--data-root <copy> daemon` on its own copy of the book (or a
fresh empty data root), with its own XDG directories and preference files
under --out and KASSIBER_NO_EGRESS=1. HOME is inherited; XDG_* are always
redirected, so neither child resolves the account's real app-data root. The
default set is every renderer-allowed, non-streaming kind of the `read`
capability that the Rust daemon advertises; --extra adds kinds such as the
report reads (capability `operator`), --kinds replaces the set, and
--args-file gives per-kind arguments ({kind: args}). Requests run in one
session per side, in order, so a mutation followed by a read compares both.

The record's request id and a few volatile keys are masked and each side's
root is written as <root>. `ui.logs.snapshot` differs by construction (each
side logs its own lines). --python-src runs the Python daemon from another
source tree (for example `git archive <commit> kassiber | tar -x`), so a
baseline run shows which differences predate a merge.

Writes <out>/report.json: the Rust and Python kind counts, the kinds Rust
does not serve, and one entry per requested kind with every differing path.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
CAPS = json.loads((REPO / "crates/runtime/tests/data/capabilities.json").read_text())["daemon"]
KINDS = json.loads((REPO / "tests/fixtures/rust_port/daemon_kinds.json").read_text())["requests"]

VOLATILE_KEYS = {
    "generated_at", "checked_at", "now", "as_of_now", "elapsed_ms", "duration_ms", "started_at",
    "finished_at", "pid", "version", "uptime_seconds", "server_time",
}


class Daemon:
    def __init__(self, argv, env, cwd):
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env=env, cwd=cwd)
        self.lines: queue.Queue = queue.Queue()
        self.stderr: list[bytes] = []
        threading.Thread(target=self._pump, daemon=True).start()
        threading.Thread(target=lambda: self.stderr.extend(self.proc.stderr), daemon=True).start()

    def _pump(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def read(self, timeout):
        line = self.lines.get(timeout=timeout)
        if line is None:
            raise EOFError(b"".join(self.stderr[-20:]).decode(errors="replace"))
        return json.loads(line)

    def call(self, request_id, kind, args, timeout=120):
        self.proc.stdin.write((json.dumps({"request_id": request_id, "kind": kind, "args": args}) + "\n").encode())
        self.proc.stdin.flush()
        records = []
        deadline = time.monotonic() + timeout
        while True:
            record = self.read(max(0.1, deadline - time.monotonic()))
            if record.get("request_id") != request_id:
                continue
            records.append(record)
            if record.get("kind") in (kind, "error") and not record.get("event"):
                return records

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=20)
        except Exception:
            self.proc.kill()


def side_env(base: Path) -> dict:
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "TMPDIR") if key in os.environ}
    for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        path = base / "xdg" / key.lower()
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    env.update({
        "KASSIBER_NO_EGRESS": "1", "KASSIBER_DISABLE_UPDATE_CHECK": "1", "PYTHONHASHSEED": "0",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(base / "offline.json"),
        "KASSIBER_AGENT_ACCESS_FILE": str(base / "agents.json"),
        "KASSIBER_UPDATE_PREFERENCE_FILE": str(base / "updates.json"),
        "KASSIBER_UPDATE_CACHE_FILE": str(base / "update-cache.json"),
    })
    return env


def normalize(value, root: str):
    if isinstance(value, dict):
        return {k: ("<volatile>" if k in VOLATILE_KEYS else normalize(v, root)) for k, v in value.items()
                if k != "request_id"}
    if isinstance(value, list):
        return [normalize(v, root) for v in value]
    if isinstance(value, str):
        return value.replace(root, "<root>")
    return value


def first_diff(a, b, path="$"):
    if type(a) is not type(b):
        return path, a, b
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                return f"{path}.{key}", a.get(key, "<missing>"), b.get(key, "<missing>")
            found = first_diff(a[key], b[key], f"{path}.{key}")
            if found:
                return found
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}[len]", len(a), len(b)
        for i, (x, y) in enumerate(zip(a, b)):
            found = first_diff(x, y, f"{path}[{i}]")
            if found:
                return found
        return None
    return None if a == b else (path, a, b)


def all_diffs(a, b, path="$", out=None, limit=60):
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if type(a) is not type(b):
        out.append((path, a, b))
    elif isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                out.append((f"{path}.{key}", a.get(key, "<missing>"), b.get(key, "<missing>")))
            else:
                all_diffs(a[key], b[key], f"{path}.{key}", out, limit)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append((f"{path}[len]", len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            all_diffs(x, y, f"{path}[{i}]", out, limit)
    elif a != b:
        out.append((path, a, b))
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rust-bin", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--book")
    parser.add_argument("--kinds")
    parser.add_argument("--args-file", help="JSON {kind: args} overrides")
    parser.add_argument("--extra", default="", help="comma-separated kinds added to the read set")
    parser.add_argument("--python-src", help="run the Python daemon from this source tree")
    args = parser.parse_args()
    out = Path(args.out).resolve()
    if out.exists():
        shutil.rmtree(out)
    overrides = json.loads(Path(args.args_file).read_text()) if args.args_file else {}
    daemons, roots = {}, {}
    for side in ("python", "rust"):
        base = out / side
        root = base / "data"
        if args.book:
            shutil.copytree(args.book, root, symlinks=True)
        else:
            root.mkdir(parents=True)
        roots[side] = str(base)
        cli = [sys.executable, "-m", "kassiber"] if side == "python" else [str(Path(args.rust_bin).resolve())]
        cwd = Path(args.python_src).resolve() if side == "python" and args.python_src else REPO
        daemons[side] = Daemon([*cli, "--data-root", str(root), "daemon"], side_env(base), cwd)
    ready = {side: d.read(120) for side, d in daemons.items()}
    native = set(ready["rust"]["data"]["supported_kinds"])
    python_kinds = ready["python"]["data"]["supported_kinds"]
    if args.kinds:
        selected = args.kinds.split(",")
    else:
        selected = [r["kind"] for r in KINDS if r["tauri_allowed"] and not r["tauri_streaming"]
                    and CAPS.get(r["kind"]) == "read" and r["kind"] in native]
        selected += [k for k in args.extra.split(",") if k and k in native and k not in selected]
    report = {"native_count": len(native), "python_count": len(python_kinds),
              "not_native": sorted(set(python_kinds) - native), "results": {}}
    for index, kind in enumerate(selected):
        request_args = overrides.get(kind, {})
        got = {}
        for side, daemon in daemons.items():
            try:
                records = daemon.call(f"r{index}", kind, request_args)
                got[side] = normalize(records[-1], roots[side])
            except Exception as exc:  # noqa: BLE001
                got[side] = {"harness_error": repr(exc)[:500]}
        diff = first_diff(got["python"], got["rust"])
        status = "same" if diff is None else "differs"
        error = got["python"].get("error")
        entry = {"status": status, "python_kind": got["python"].get("kind"), "rust_kind": got["rust"].get("kind"),
                 "python_error": error.get("message") if isinstance(error, dict) else None}
        if diff:
            entry["first_diff"] = [diff[0], json.dumps(diff[1])[:300], json.dumps(diff[2])[:300]]
            entry["diffs"] = [[p, json.dumps(x)[:200], json.dumps(y)[:200]] for p, x, y in all_diffs(got["python"], got["rust"])]
            entry["python"] = got["python"]
            entry["rust"] = got["rust"]
        report["results"][kind] = entry
        print(f"{status:8} {kind} {entry.get('first_diff', '')}"[:400], flush=True)
    for daemon in daemons.values():
        daemon.close()
    (out / "report.json").write_text(json.dumps(report, indent=1, sort_keys=True))
    counts = {}
    for entry in report["results"].values():
        counts[entry["status"]] = counts.get(entry["status"], 0) + 1
    print(json.dumps({"native": len(native), "python": len(python_kinds), **counts}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

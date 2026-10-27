"""Replay a recorded session against a candidate and compare the two runs.

The candidate is the Python reference unless told otherwise. It starts from
a fresh copy of the recorded starting state, at the recorded paths when they
are free and at fresh temporary paths (mapped both ways) when they are not,
and receives the same argv, environment subset, stdin, and fixture files in
the recorded order: a stdin write, a signal, or the next process waits until
the candidate has printed as many lines as the reference had at that point.
Identities a candidate printed earlier are translated into later inputs, as
a test echoing an id back would do.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

from tests import cli_runner
from tests.session_harness import dbdump, policy
from tests.session_harness.compare import Difference, Mapper, Window, compare_file, compare_tree
from tests.session_harness.recorder import decode_bytes
from tests.session_harness.snapshot import BlobStore, capture_tree, restore_tree


ROOT = Path(__file__).resolve().parents[2]
CHILD_HOOKS = ROOT / "tests" / "_child_hooks"
OUTPUT_WAIT_SECONDS = 60.0
STEP_TIMEOUT_SECONDS = 300.0
# Differences beyond this many per session are counted, not listed.
MAX_LISTED = 200


@dataclass
class Report:
    session: str
    status: str
    differences: list[Difference] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)
    steps: int = 0
    seconds: float = 0.0
    relocated: dict[str, str] = field(default_factory=dict)
    tests: list[str] = field(default_factory=list)
    total_differences: int = 0
    # Differences left under each diagnostic relaxation (`compare.Mapper`).
    diagnostics: dict[str, int] = field(default_factory=dict)
    # What is left with clock readings and entropy relaxed: the differences
    # injection would not remove.
    residual: list[Difference] = field(default_factory=list)

    def as_dict(self) -> dict:
        return {
            "session": self.session,
            "status": self.status,
            "steps": self.steps,
            "seconds": round(self.seconds, 2),
            "tests": self.tests,
            "notes": self.notes,
            "relocated": self.relocated,
            "total_differences": self.total_differences,
            "diagnostics": self.diagnostics,
            "residual": [difference.as_dict() for difference in self.residual],
            "differences": [difference.as_dict() for difference in self.differences],
        }


def load_events(session_dir: Path) -> list[dict]:
    events = []
    with open(session_dir / "events.jsonl", encoding="utf-8") as handle:
        for line in handle:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                # A recording interrupted mid-write ends with a partial line.
                break
    return events


class _Proc:
    def __init__(self, popen: subprocess.Popen, api: str):
        self.popen = popen
        self.api = api
        self.out = bytearray()
        self.err = bytearray()
        self.lines = 0
        self.compared_lines = 0
        self.eof = False
        self.cond = threading.Condition()
        self.readers = [threading.Thread(target=self._read, args=(popen.stdout, self.out, True), daemon=True)]
        if popen.stderr is not None:
            self.readers.append(threading.Thread(target=self._read, args=(popen.stderr, self.err, False), daemon=True))
        for reader in self.readers:
            reader.start()

    def _read(self, stream, buffer: bytearray, count: bool) -> None:
        try:
            while chunk := os.read(stream.fileno(), 65536):
                with self.cond:
                    buffer.extend(chunk)
                    if count:
                        self.lines += chunk.count(b"\n")
                    self.cond.notify_all()
        except OSError:
            pass
        finally:
            with self.cond:
                if count:
                    self.eof = True
                self.cond.notify_all()

    def wait_lines(self, count: int, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        with self.cond:
            while self.lines < count and not self.eof:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return False
                self.cond.wait(remaining)
            return self.lines >= count


class Replay:
    def __init__(self, session_dir: Path, candidate: tuple[str, ...] | None, *, keep: bool = False,
                 step_timeout: float = STEP_TIMEOUT_SECONDS, extra_env: dict[str, str] | None = None,
                 diagnose: bool = False):
        self.dir = Path(session_dir)
        self.manifest = json.loads((self.dir / "manifest.json").read_text(encoding="utf-8"))
        self.store = BlobStore(self.dir / "blobs")
        self.reference = candidate is None
        self.prefix = candidate or cli_runner.REFERENCE_PREFIX
        self.keep = keep
        self.step_timeout = step_timeout
        self.extra_env = dict(extra_env or {})
        self.diagnose = diagnose
        self.differences: list[Difference] = []
        self.notes: list[str] = []
        self.procs: dict[int, _Proc] = {}
        self.ref_out: dict[int, dict[str, bytearray]] = {}
        self.cleanup: list[Path] = []
        self.relocations: dict[str, str] = {}
        self.states: dict[str, dict] = {}

    # Paths ----------------------------------------------------------------

    def _prepare_paths(self) -> None:
        """Use the recorded paths where this replay can claim them, else fresh ones.

        A replay claims the recorded directory directly under the temp root
        by creating it (atomic), so parallel replays whose recordings share
        a directory, such as the suite's offline-preference file, never
        share it: the later ones relocate.
        """
        temp = policy.temp_root()
        claimed: set[Path] = set()
        for anchor in self.manifest.get("anchors", []):
            path = Path(anchor)
            top = Path(temp, path.relative_to(temp).parts[0]) if policy.in_temp_area(path) else None
            if top is not None and top not in claimed:
                try:
                    os.mkdir(top, 0o700)
                except OSError:
                    pass
                else:
                    claimed.add(top)
                    self.cleanup.append(top)
            if top in claimed and (path == top or not os.path.lexists(path)):
                continue
            fresh = tempfile.mkdtemp(prefix="kassiber-replay-")
            os.rmdir(fresh)
            self.relocations[anchor] = fresh
            self.cleanup.append(Path(fresh))

    def loc(self, path: str) -> Path:
        for ref, cand in sorted(self.relocations.items(), key=lambda item: -len(item[0])):
            if path == ref or path.startswith(ref.rstrip("/") + "/"):
                return Path(cand + path[len(ref):])
        return Path(path)

    def state(self, state_id: str) -> dict:
        if state_id not in self.states:
            self.states[state_id] = json.loads((self.dir / "states" / f"{state_id}.json").read_text(encoding="utf-8"))
        return self.states[state_id]

    def _restore_roots(self, state_id: str, roots=None) -> None:
        for root_id, tree in self.state(state_id).items():
            if roots is not None and root_id not in roots:
                continue
            destination = self.loc(self.manifest["roots"][root_id]["path"])
            if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
                shutil.rmtree(destination)
            destination.parent.mkdir(parents=True, exist_ok=True)
            restore_tree(tree, self.store, destination)
            self._pin_tree(tree)

    # Every comparison runs in each lane: the strict mapper decides, the
    # relaxed ones (with `diagnose`) only say what the differences are made of.

    def _pin(self, text: str) -> None:
        for _label, mapper, _out in self.lanes:
            mapper.pin_text(text)

    def _everywhere(self, difference: Difference) -> None:
        for _label, _mapper, out in self.lanes:
            out.append(difference)

    def _pin_tree(self, tree: dict) -> None:
        for rel, entry in (tree.get("entries") or {}).items():
            self._pin(rel)
            if entry.get("type") != "file":
                continue
            self._pin_bytes(self.store.get(entry["sha256"]))

    def _pin_bytes(self, data: bytes) -> None:
        if dbdump.is_sqlite(data):
            try:
                dump = dbdump.dump_files(data)
            except dbdump.UnreadableDatabase:
                return
            for table in dump.tables.values():
                for _rowid, row in table.rows:
                    for value in row:
                        if isinstance(value, str):
                            self._pin(value)
            return
        if b"\x00" in data:
            return
        try:
            self._pin(data.decode("utf-8"))
        except UnicodeDecodeError:
            pass

    # Running --------------------------------------------------------------

    def run(self) -> Report:
        started = time.time()
        manifest = self.manifest
        report = Report(session=self.dir.name, status="clean", tests=manifest.get("tests", []))
        if manifest.get("refused"):
            report.status = "refused"
            report.notes.append(manifest["refused"])
            return report
        if manifest.get("format") != policy.SESSION_FORMAT or manifest.get("comparison_policy") != policy.COMPARISON_POLICY:
            report.status = "error"
            report.notes.append(
                f"recorded as {manifest.get('format')} under {manifest.get('comparison_policy')}; "
                f"this harness replays {policy.SESSION_FORMAT} under {policy.COMPARISON_POLICY}"
            )
            return report
        wall = manifest.get("wall") or {}
        reference = Window(wall.get("start") or 0.0, wall.get("end") or wall.get("start") or 0.0)
        self.mapper = Mapper(reference, Window(started, started))
        self.lanes = [("strict", self.mapper, self.differences)]
        if self.diagnose:
            for label, relax in (
                ("clock", {"clock"}),
                ("clock+entropy", {"clock", "entropy"}),
                ("clock+entropy+order", {"clock", "entropy", "order"}),
            ):
                self.lanes.append((label, Mapper(reference, Window(started, started), relax=frozenset(relax)), []))
        try:
            self._prepare_paths()
            self._set_relocations()
            events = load_events(self.dir)
            keys = self._database_keys(events)
            for _label, mapper, _out in self.lanes:
                mapper.database_keys = keys
            if manifest["states"].get("start"):
                self._restore_roots(manifest["states"]["start"])
            for event in events:
                self._set_window(started)
                self._handle(event)
            for proc_id, proc in self.procs.items():
                if proc.popen.poll() is None:
                    proc.popen.kill()
                    proc.popen.wait()
                    self.notes.append(f"proc {proc_id} had no recorded exit; stopped")
            self._set_window(started)
            if manifest["states"].get("end"):
                self._compare_roots(manifest["states"]["end"], "end")
        except Exception as exc:  # A harness failure must not read as a clean replay.
            report.status = "error"
            self.notes.append(f"replay error: {type(exc).__name__}: {exc}")
        finally:
            for proc in self.procs.values():
                if proc.popen.poll() is None:
                    proc.popen.kill()
                    proc.popen.wait()
            if not self.keep:
                for path in self.cleanup:
                    shutil.rmtree(path, ignore_errors=True)
        report.differences = self.differences[:MAX_LISTED]
        report.total_differences = len(self.differences)
        report.diagnostics = {label: len(out) for label, _mapper, out in self.lanes[1:]}
        if len(self.lanes) > 1:
            report.residual = self.lanes[-1][2][:MAX_LISTED]
        report.notes.extend(self.notes)
        report.steps = manifest.get("steps", 0)
        report.seconds = time.time() - started
        report.relocated = dict(self.relocations)
        if manifest.get("incomplete"):
            report.notes.extend(f"recording incomplete: {item}" for item in manifest["incomplete"])
        report.notes.extend(f"recorded: {item}" for item in manifest.get("notes", []))
        if report.status != "error":
            if self.differences:
                report.status = "different"
            elif manifest.get("incomplete"):
                report.status = "incomplete"
        return report

    def _database_keys(self, events: list[dict]) -> list[str]:
        """Synthetic passphrases the session fed in, to open the books they encrypted."""
        keys: list[str] = []
        for event in events:
            if event["event"] == "spawn":
                keys.extend(
                    self.store.get(arg["fd_input"]).decode("utf-8", "replace").removesuffix("\n")
                    for arg in event["args"]
                    if isinstance(arg, dict) and "fd_input" in arg
                )
            elif event["event"] == "stdin" and "text" in event:
                for line in event["text"].splitlines():
                    try:
                        keys.extend(_passphrases(json.loads(line)))
                    except ValueError:
                        continue
        return list(dict.fromkeys(keys))

    def _set_window(self, started: float) -> None:
        for _label, mapper, _out in self.lanes:
            mapper.set_candidate_window(Window(started, time.time()))

    def _set_relocations(self) -> None:
        ordered = sorted(self.relocations.items(), key=lambda pair: -len(pair[0]))
        for _label, mapper, _out in self.lanes:
            mapper.relocations = ordered

    def _handle(self, event: dict) -> None:
        kind = event["event"]
        proc_id = event.get("proc")
        if kind == "spawn":
            self._spawn(event)
        elif kind in ("stdout", "stderr"):
            self.ref_out.setdefault(proc_id, {"stdout": bytearray(), "stderr": bytearray()})[kind].extend(decode_bytes(event))
        elif kind == "stdin":
            self._wait_after(event.get("after", {}))
            for item in event.get("inputs", []):
                self._materialize(item)
            data = decode_bytes(event)
            try:
                text = data.decode("utf-8")
            except UnicodeDecodeError:
                payload = data
            else:
                self._pin(text)
                payload = self.mapper.translate(text).encode("utf-8")
            proc = self.procs.get(proc_id)
            if proc is not None and proc.popen.stdin is not None:
                try:
                    proc.popen.stdin.write(payload)
                    proc.popen.stdin.flush()
                except OSError:
                    self.notes.append(f"proc {proc_id}: stdin closed early")
        elif kind == "stdin_close":
            self._wait_after(event.get("after", {}))
            proc = self.procs.get(proc_id)
            if proc is not None and proc.popen.stdin is not None:
                try:
                    proc.popen.stdin.close()
                except OSError:
                    pass
        elif kind == "signal":
            self._wait_after(event.get("after", {}))
            proc = self.procs.get(proc_id)
            if proc is not None and proc.popen.poll() is None:
                proc.popen.send_signal(event["signal"])
        elif kind == "exit":
            self._exit(event)
        elif kind == "started":
            proc = self.procs.get(proc_id)
            if proc is not None:
                for _label, mapper, _out in self.lanes:
                    mapper.bind_pid(event["pid"], proc.popen.pid)
        elif kind == "note":
            pass
        elif kind == "spawn_failed":
            self.notes.append(f"proc {proc_id}: the reference failed to start ({event.get('error')})")

    def _wait_after(self, after: dict) -> None:
        for key, count in after.items():
            proc = self.procs.get(int(key))
            if proc is None:
                continue
            if not proc.wait_lines(count, OUTPUT_WAIT_SECONDS):
                self._everywhere(
                    Difference(f"proc {key} stdout", "timeout", f"candidate printed {proc.lines} lines; the reference had {count} by now")
                )
            self._compare_lines(int(key), count)

    def _compare_lines(self, proc_id: int, upto: int | None) -> None:
        proc = self.procs[proc_id]
        if proc.api != "popen":
            return
        ref_lines = bytes(self.ref_out.get(proc_id, {}).get("stdout", b"")).splitlines(keepends=True)
        with proc.cond:
            cand_lines = bytes(proc.out).splitlines(keepends=True)
        if upto is None:
            if len(ref_lines) != len(cand_lines):
                self._everywhere(
                    Difference(f"proc {proc_id} stdout", "length", "line count differs", str(len(ref_lines)), str(len(cand_lines)))
                )
            upto = max(len(ref_lines), len(cand_lines))
        end = min(upto, len(ref_lines), len(cand_lines))
        for index in range(proc.compared_lines, end):
            left, right = ref_lines[index], cand_lines[index]
            where = f"proc {proc_id} stdout#{index + 1}"
            try:
                left_text, right_text = left.decode("utf-8"), right.decode("utf-8")
            except UnicodeDecodeError:
                if left != right:
                    self._everywhere(Difference(where, "bytes", "line differs"))
                continue
            for _label, mapper, out in self.lanes:
                mapper.compare_line(left_text, right_text, where, out)
        proc.compared_lines = max(proc.compared_lines, end)

    def _spawn(self, event: dict) -> None:
        self._wait_after(event.get("after", {}))
        if "segment" in event:
            # The test changed the state between processes; compare what the
            # candidate left, then continue from what the test left.
            self._compare_roots(event["segment"]["end"], f"segment before step {event['proc']}")
            for _label, mapper, _out in self.lanes:
                mapper.reset()
            self._restore_roots(event["segment"]["start"])
        if "added_roots" in event:
            self._restore_roots(event["added_roots"]["state"], roots=set(event["added_roots"]["roots"]))
        for item in event.get("inputs", []):
            self._materialize(item)
        pass_fds = []
        args = []
        for arg in event["args"]:
            if isinstance(arg, dict) and "fd_input" in arg:
                read_fd, write_fd = os.pipe()
                os.write(write_fd, self.store.get(arg["fd_input"]))
                os.close(write_fd)
                pass_fds.append(read_fd)
                args.append(str(read_fd))
                continue
            self._pin(arg)
            args.append(self.mapper.translate(arg))
        cwd = event["cwd"]
        cwd_path = ROOT / cwd["repo"] if "repo" in cwd else self.loc(cwd["path"])
        stdio = event["stdio"]
        if stdio["stdin"] == "inherit" and stdio.get("stdin_isatty"):
            self.notes.append(f"proc {event['proc']}: stdin was a terminal when recorded; replayed from /dev/null")
        stdin = subprocess.DEVNULL if stdio["stdin"] in ("devnull", "inherit") else subprocess.PIPE
        popen = subprocess.Popen(
            [*self.prefix, *args],
            cwd=cwd_path,
            env=self._env(event["env"]),
            stdin=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT if stdio["stderr"] == "stdout" else subprocess.PIPE,
            pass_fds=tuple(pass_fds),
            start_new_session=bool(event.get("start_new_session")),
        )
        for fd in pass_fds:
            os.close(fd)
        self.procs[event["proc"]] = _Proc(popen, event["api"])

    def _env(self, recorded: dict) -> dict[str, str]:
        env = {"PATH": os.environ.get("PATH", os.defpath)}
        for name in ("SYSTEMROOT", "COMSPEC", "PATHEXT", "WINDIR"):
            if name in os.environ:
                env[name] = os.environ[name]
        for name, spec in recorded.items():
            if "value" in spec:
                self._pin(spec["value"])
                env[name] = self.mapper.translate(spec["value"])
            elif "path" in spec:
                env[name] = str(self.loc(spec["path"]))
            elif "repo" in spec:
                env[name] = str(ROOT / spec["repo"])
            elif "inherit" in spec:
                if name in os.environ:
                    env[name] = os.environ[name]
            elif "test" in spec and self.reference:
                env[name] = spec["test"]
        if self.reference:
            # The Python reference replays under the suite's own guard.
            env["PYTHONPATH"] = os.pathsep.join((str(CHILD_HOOKS), str(ROOT)))
            env.setdefault("KASSIBER_TEST_NO_EGRESS", "1")
        env.update(self.extra_env)
        return env

    def _materialize(self, item: dict) -> None:
        if item.get("produced"):
            return
        if "repo" in item:
            path = ROOT / item["repo"]
            if "blob" in item and (not path.is_file() or path.read_bytes() != self.store.get(item["blob"])):
                # The fixture changed since recording: hand the recorded bytes
                # over at a fresh path instead of touching the checkout.
                fresh = Path(tempfile.mkdtemp(prefix="kassiber-replay-fixture-")) / path.name
                fresh.write_bytes(self.store.get(item["blob"]))
                self.cleanup.append(fresh.parent)
                self.relocations[str(path)] = str(fresh)
                self._set_relocations()
            if "blob" in item:
                self._pin_bytes(self.store.get(item["blob"]))
            return
        destination = self.loc(item["path"])
        if "blob" in item:
            data = self.store.get(item["blob"])
            self._pin_bytes(data)
            if destination.is_file() and destination.read_bytes() == data:
                return
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        elif "tree" in item:
            if destination.exists():
                shutil.rmtree(destination)
            destination.parent.mkdir(parents=True, exist_ok=True)
            restore_tree(item["tree"], self.store, destination)
            self._pin_tree(item["tree"])
        elif item.get("absent"):
            destination.parent.mkdir(parents=True, exist_ok=True)

    def _exit(self, event: dict) -> None:
        proc_id = event["proc"]
        proc = self.procs.get(proc_id)
        if proc is None:
            return
        where = f"proc {proc_id}"
        try:
            returncode = proc.popen.wait(timeout=self.step_timeout)
        except subprocess.TimeoutExpired:
            proc.popen.kill()
            returncode = proc.popen.wait()
            self._everywhere(Difference(where, "timeout", f"candidate still running after {self.step_timeout:.0f}s"))
        for reader in proc.readers:
            reader.join(timeout=30)
        recorded = self.ref_out.get(proc_id, {"stdout": bytearray(), "stderr": bytearray()})
        if proc.api == "popen":
            self._compare_lines(proc_id, None)
        for _label, mapper, out in self.lanes:
            if proc.api != "popen":
                mapper.compare_stream(bytes(recorded["stdout"]), bytes(proc.out), f"{where} stdout", out)
            mapper.compare_stream(bytes(recorded["stderr"]), bytes(proc.err), f"{where} stderr", out)
        if returncode != event["returncode"]:
            self._everywhere(Difference(where, "exit", "exit status differs", str(event["returncode"]), str(returncode)))
        for path, expected in event.get("outputs", {}).items():
            self._compare_output(path, expected, f"{where} file {path}")

    def _compare_output(self, path: str, expected: dict, where: str) -> None:
        actual = self.loc(path)
        if expected.get("absent"):
            if actual.exists():
                self._everywhere(Difference(where, "extra", "the candidate created a file the reference did not"))
            return
        if "blob" in expected:
            if not actual.is_file():
                self._everywhere(Difference(where, "missing", "the candidate did not write this file"))
                return
            data = actual.read_bytes()
            for _label, mapper, out in self.lanes:
                compare_file(self.store.get(expected["blob"]), data, where, mapper, out)
            return
        tree = capture_tree(actual)
        for _label, mapper, out in self.lanes:
            compare_tree(
                expected["tree"],
                lambda rel: self.store.get(expected["tree"]["entries"][rel]["sha256"]),
                tree,
                lambda rel: (actual / rel).read_bytes(),
                mapper,
                where,
                out,
            )

    def _compare_roots(self, state_id: str, label: str) -> None:
        for root_id, ref_tree in self.state(state_id).items():
            root = self.manifest["roots"][root_id]
            actual = self.loc(root["path"])
            cand_tree = capture_tree(actual)
            for _lane, mapper, out in self.lanes:
                compare_tree(
                    ref_tree,
                    lambda rel, tree=ref_tree: self.store.get(tree["entries"][rel]["sha256"]),
                    cand_tree,
                    lambda rel, base=actual: (base / rel).read_bytes(),
                    mapper,
                    f"{label} {root['kind']}",
                    out,
                )


def _passphrases(value) -> list[str]:
    found = []
    if isinstance(value, dict):
        for key, item in value.items():
            if key in policy.PASSPHRASE_FIELDS and isinstance(item, str) and item:
                found.append(item)
            else:
                found.extend(_passphrases(item))
    elif isinstance(value, list):
        for item in value:
            found.extend(_passphrases(item))
    return found


def replay(session_dir, candidate: tuple[str, ...] | None = None, **options) -> Report:
    return Replay(Path(session_dir), candidate, **options).run()


def session_dirs(paths) -> list[Path]:
    """Session directories under each path (a recording root or a session)."""
    found = []
    for path in map(Path, paths):
        if (path / "manifest.json").is_file():
            found.append(path)
        elif (path / "sessions").is_dir():
            found.extend(sorted(child for child in (path / "sessions").iterdir() if (child / "manifest.json").is_file()))
    return found


"""Record what product child processes saw and did, at the process boundary.

`tests/cli_runner.py` hands every invocation here while
`KASSIBER_TEST_RECORD_SESSIONS=<dir>` is set. A session is everything the
tests ran against one state root in the test temp area: its starting tree,
then each process in order (argv, working directory, the product's
environment subset, stdin, fixture files it named, stdout, stderr, signals,
and its exit status), and the tree whenever the session falls idle.
Children talk to the test through pipes this module pumps, so a test sees
real pipes and the recorder sees every byte in order.

Nothing here runs inside the product: the child is the unmodified command.
The recorder writes only under its own directory, refuses roots outside the
temp area, never keeps secret-named environment values, refuses a session
whose bytes contain a secret value from the test's environment, and refuses
stdin a call site marks as a credential.
"""

from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from tests.session_harness import policy
from tests.session_harness.snapshot import BlobStore, capture_tree, tree_blobs


ROOT = Path(__file__).resolve().parents[2]
_RECORDERS: dict[Path, "Recorder"] = {}
_RECORDERS_LOCK = threading.Lock()
_JSON_STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')
# How long a test's wait may be held for the exit to be recorded.
_JOIN_SECONDS = 60.0


class Refused(Exception):
    """This invocation or session must not be recorded."""


def recorder_for(directory: str | os.PathLike[str]) -> "Recorder":
    path = Path(directory).expanduser().resolve()
    with _RECORDERS_LOCK:
        recorder = _RECORDERS.get(path)
        if recorder is None:
            recorder = _RECORDERS[path] = Recorder(path)
            _watch_test_database_access()
        return recorder


def _watch_test_database_access() -> None:
    """Note when the test process opens a session's database during a live process.

    Such a write (a test seeding a row between daemon requests) happens
    outside every recorded process, so a replay cannot repeat it; the note
    names the cause in the replay report.
    """
    import sqlite3

    modules = [sqlite3]
    try:
        import sqlcipher3

        modules.append(sqlcipher3)
    except ImportError:
        pass
    for module in modules:
        original = module.connect
        if getattr(original, "_kassiber_session_watch", False):
            continue

        def connect(database, *args, _original=original, **kwargs):
            connection = _original(database, *args, **kwargs)
            try:
                _note_database_access(database)
            except Exception:  # Never let the note change what the test sees.
                pass
            return connection

        connect._kassiber_session_watch = True
        module.connect = connect


def _note_database_access(database) -> None:
    text = os.fspath(database) if not isinstance(database, bytes) else database.decode()
    if text.startswith("file:"):
        text = text[len("file:"):].split("?", 1)[0]
    if not text or text == ":memory:":
        return
    path = Path(text).resolve()
    for recorder in list(_RECORDERS.values()):
        for session in list(recorder.sessions.values()):
            if session.live and any(policy.inside(path, Path(root["path"])) for root in session.roots.values()):
                session.note(f"the test process opened {path.name} while a recorded process was live")


def _current_test() -> str:
    current = os.environ.get("PYTEST_CURRENT_TEST", "")
    return current.rsplit(" (", 1)[0] if current else ""


def encode_bytes(data: bytes) -> dict:
    try:
        return {"text": data.decode("utf-8")}
    except UnicodeDecodeError:
        return {"base64": base64.b64encode(data).decode("ascii")}


def decode_bytes(value: dict) -> bytes:
    if "text" in value:
        return value["text"].encode("utf-8")
    return base64.b64decode(value["base64"])


def _git(*args: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", *args], cwd=ROOT, capture_output=True, text=True, timeout=30, check=False
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def reference_provenance() -> dict:
    """The reference build a recording comes from."""
    import kassiber

    try:
        import kassiber_tax

        engine = kassiber_tax.engine_version()
    except Exception:  # pragma: no cover - the binding is a hard dependency
        engine = None
    return {
        "git_commit": _git("rev-parse", "HEAD"),
        "git_dirty": bool(_git("status", "--porcelain", "--untracked-files=no")),
        "kassiber_version": kassiber.__version__,
        "tax_engine_version": engine,
        "schema_fingerprint": schema_fingerprint(),
        "generator": "python -m kassiber",
        "python": platform.python_version(),
        "platform": sys.platform,
    }


def schema_fingerprint() -> str:
    """sha256 of the schema objects a fresh book gets."""
    from kassiber.db import open_db

    with tempfile.TemporaryDirectory(prefix="kassiber-session-schema-") as scratch:
        conn = open_db(Path(scratch) / "data")
        try:
            rows = conn.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name"
            ).fetchall()
        finally:
            conn.close()
    digest = hashlib.sha256(json.dumps([list(row) for row in rows]).encode("utf-8"))
    return f"sha256:{digest.hexdigest()}"


class Recorder:
    def __init__(self, directory: Path):
        self.directory = directory
        self.lock = threading.Lock()
        self.sessions: dict[Path, Session] = {}
        self._provenance: dict | None = None
        self._next_session = 0
        self.secret_values = sorted(
            {
                value.encode("utf-8")
                for name, value in os.environ.items()
                if policy.is_secret_name(name) and len(value) >= policy.SECRET_SCAN_MIN_LENGTH
            },
            key=len,
            reverse=True,
        )

    @property
    def provenance(self) -> dict:
        with self.lock:
            if self._provenance is None:
                self._provenance = reference_provenance()
            return self._provenance

    def refuse(self, reason: str) -> None:
        self.directory.mkdir(parents=True, exist_ok=True)
        line = json.dumps({"test": _current_test(), "reason": reason}) + "\n"
        with self.lock, open(self.directory / "refusals.jsonl", "a", encoding="utf-8") as handle:
            handle.write(line)

    # The two entry points mirror subprocess.run and subprocess.Popen.

    def run(self, command, *, product_args, python_options, secret_input, fd_inputs=(), **kwargs):
        input_data = kwargs.pop("input", None)
        capture_output = kwargs.pop("capture_output", False)
        timeout = kwargs.pop("timeout", None)
        check = kwargs.pop("check", False)
        session = self._session_for(command, product_args, python_options, secret_input, fd_inputs, kwargs)
        if session is None:
            if input_data is not None:
                kwargs["input"] = input_data
            return subprocess.run(command, capture_output=capture_output, timeout=timeout, check=check, **kwargs)
        if input_data is not None:
            if kwargs.get("stdin") is not None:
                raise ValueError("stdin and input arguments may not both be used.")
            kwargs["stdin"] = subprocess.PIPE
        if capture_output:
            if kwargs.get("stdout") is not None or kwargs.get("stderr") is not None:
                raise ValueError("stdout and stderr arguments may not be used with capture_output.")
            kwargs["stdout"] = subprocess.PIPE
            kwargs["stderr"] = subprocess.PIPE
        process = session.spawn(command, product_args, fd_inputs, api="run", **kwargs)
        with process:
            try:
                stdout, stderr = process.communicate(input_data, timeout=timeout)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                raise
            except BaseException:
                process.kill()
                raise
            retcode = process.poll()
        process.join_recording()
        if check and retcode:
            raise subprocess.CalledProcessError(retcode, process.args, output=stdout, stderr=stderr)
        return subprocess.CompletedProcess(process.args, retcode, stdout, stderr)

    def popen(self, command, *, product_args, python_options, secret_input, fd_inputs=(), **kwargs):
        session = self._session_for(command, product_args, python_options, secret_input, fd_inputs, kwargs)
        if session is None:
            return subprocess.Popen(command, **kwargs)
        return session.spawn(command, product_args, fd_inputs, api="popen", **kwargs)

    def _session_for(self, command, product_args, python_options, secret_input, fd_inputs, kwargs):
        try:
            if os.environ.get("KASSIBER_TEST_CLI", "").strip():
                raise Refused("recordings come from the Python reference, and KASSIBER_TEST_CLI is set")
            if python_options:
                raise Refused("interpreter options instrument the reference")
            if secret_input:
                raise Refused("stdin carries a credential")
            synthetic = {int(product_args[index]) for index, _data in fd_inputs}
            if set(kwargs.get("pass_fds") or ()) - synthetic:
                raise Refused("inherited descriptors without recorded content")
            if kwargs.get("shell"):
                raise Refused("shell invocation")
            env = kwargs.get("env")
            env = dict(os.environ if env is None else env)
            for name in policy.REFUSED_TEST_ENV:
                if env.get(name):
                    raise Refused(f"{name} alters the reference's outcome")
            cwd = Path(kwargs.get("cwd") or os.getcwd()).resolve()
            primary, _roots = self._roots(product_args, env, cwd)
            with self.lock:
                session = self.sessions.get(primary)
                if session is None:
                    self._next_session += 1
                    session = Session(self, primary, self._next_session)
                    self.sessions[primary] = session
            if session.refused:
                return None
            return session
        except Refused as exc:
            self.refuse(str(exc))
            return None

    def _roots(self, product_args, env, cwd) -> tuple[Path, list[tuple[Path, str]]]:
        """The session key and the state directories a command reads and writes.

        A data root named `data` keeps its `config`, `exports`, and
        `attachments` beside it, in the parent state root (the data-root
        layout contract); other data roots hold them inside. Only those
        product directories are state: a fixture the test writes next to
        them is an input of the command that names it.
        """
        found: list[tuple[Path, str]] = []
        key = None
        data_root = _option_value(product_args, "--data-root")
        if data_root is not None:
            path = (cwd / data_root).resolve() if not os.path.isabs(data_root) else Path(data_root).resolve()
            state_root = path.parent if path.name == policy.DATA_DIRNAME else path
            if not policy.in_temp_area(state_root):
                raise Refused("data root outside the test temp area")
            key = state_root
            if state_root == path:
                found.append((path, "data_root"))
            else:
                found.extend((state_root / name, name) for name in policy.STATE_DIRNAMES)
        for name in sorted(env):
            if not policy.is_product_env(name):
                continue
            value = env[name]
            if policy.env_path_kind(name, value) != "dir":
                continue
            path = Path(value).resolve()
            if policy.in_temp_area(path):
                found.append((path, name.lower()))
        if not found:
            raise Refused("no state root in the test temp area")
        if self.directory == policy.temp_root() or any(
            policy.inside(self.directory, path) or policy.inside(path, self.directory)
            for path in [path for path, _ in found] + ([key] if key else [])
        ):
            raise Refused("the recording directory overlaps a state root")
        outer: list[tuple[Path, str]] = []
        for path, kind in found:
            if any(policy.inside(path, other) for other, _ in found if other != path):
                continue
            if all(path != existing for existing, _ in outer):
                outer.append((path, kind))
        return key or outer[0][0], outer


def _option_value(args, option: str) -> str | None:
    for index, arg in enumerate(args):
        if not isinstance(arg, str):
            continue
        if arg == option and index + 1 < len(args) and isinstance(args[index + 1], str):
            return args[index + 1]
        if arg.startswith(option + "="):
            return arg.split("=", 1)[1]
    return None


class Session:
    def __init__(self, recorder: Recorder, primary: Path, number: int):
        self.recorder = recorder
        self.primary = primary
        self.lock = threading.RLock()
        self.refused: str | None = None
        slug = re.sub(r"[^A-Za-z0-9]+", "-", _current_test().split("::")[-1] or "session").strip("-")[:48]
        digest = hashlib.sha256(f"{os.getpid()}:{primary}".encode()).hexdigest()[:12]
        self.directory = recorder.directory / "sessions" / f"{slug or 'session'}-{digest}"
        self.directory.mkdir(parents=True, exist_ok=True)
        self.store = BlobStore(self.directory / "blobs")
        self.roots: dict[str, dict] = {}
        self.anchors: set[Path] = set()
        self.tests: list[str] = []
        self.events = open(self.directory / "events.jsonl", "a", encoding="utf-8")
        self.seq = 0
        self.next_proc = 0
        self.live: dict[int, "RecordingPopen"] = {}
        self.stdout_lines: dict[int, int] = {}
        self.exclude: set[Path] = set()
        self.started = False
        self.wall_start: float | None = None
        self.wall_end: float | None = None
        self.states: dict[str, dict] = {}
        self.state_counter = 0
        self.scanned: set[str] = set()
        self.pinned_states: list[str] = []
        self.segments: list[dict] = []
        self.start_state: str | None = None
        self.latest_end: str | None = None
        self.pinned_blobs: set[str] = set()
        self.produced: dict[str, str] = {}
        self.provenance_env: dict[str, str] = {}
        self.steps = 0
        self.incomplete: list[str] = []
        self.notes: list[str] = []

    # Recording primitives -------------------------------------------------

    def put(self, data: bytes, *, pin: bool = True) -> str:
        self.check_secrets(data)
        digest = self.store.put_bytes(data)
        if pin:
            self.pinned_blobs.add(digest)
        return digest

    def _scan_blobs(self, digests) -> None:
        # Captured trees write blobs first; each needs the secret scan once.
        for digest in sorted(set(digests) - self.scanned):
            self.check_secrets(self.store.get(digest))
            self.scanned.add(digest)

    def check_secrets(self, data: bytes) -> None:
        for secret in self.recorder.secret_values:
            if secret in data:
                raise Refused("a secret value from the test environment reached the session")

    def emit(self, event: dict) -> None:
        with self.lock:
            if self.refused:
                return
            self.seq += 1
            event = {"seq": self.seq, **event}
            line = json.dumps(event, ensure_ascii=False)
            try:
                self.check_secrets(line.encode("utf-8"))
            except Refused as exc:
                self.refuse(str(exc))
                return
            self.events.write(line + "\n")

    def after(self) -> dict[str, int]:
        return {str(proc): self.stdout_lines.get(proc, 0) for proc in sorted(self.live)}

    def record_chunk(self, proc: int, stream: str, data: bytes) -> None:
        with self.lock:
            if stream == "stdout":
                self.stdout_lines[proc] = self.stdout_lines.get(proc, 0) + data.count(b"\n")
            self.emit({"event": stream, "proc": proc, **encode_bytes(data)})

    def record_stdin(self, proc: int, data: bytes | None, *, broken: bool = False) -> None:
        with self.lock:
            if data is None:
                self.emit({"event": "stdin_close", "proc": proc, "after": self.after(), "broken": broken})
                return
            event = {"event": "stdin", "proc": proc, "after": self.after(), **encode_bytes(data)}
            try:
                inputs = self._inputs(_paths_in_text(data), Path(self.live[proc].recorded_cwd)) if proc in self.live else []
            except Refused as exc:
                self.refuse(str(exc))
                return
            if inputs:
                event["inputs"] = inputs
                self.live[proc].watched.update(item["path"] for item in inputs if item.get("watch"))
            self.emit(event)

    def note(self, text: str) -> None:
        with self.lock:
            if text not in self.notes:
                self.notes.append(text)
                self.emit({"event": "note", "text": text})
                self.write_manifest()

    def record_signal(self, proc: int, signum: int) -> None:
        self.emit({"event": "signal", "proc": proc, "signal": int(signum), "after": self.after()})

    def refuse(self, reason: str) -> None:
        """Drop everything kept for this session; keep only why."""
        with self.lock:
            if self.refused:
                return
            self.refused = reason
            self.recorder.refuse(reason)
            try:
                self.events.close()
            finally:
                shutil.rmtree(self.directory, ignore_errors=True)
                self.directory.mkdir(parents=True, exist_ok=True)
                (self.directory / "manifest.json").write_text(
                    json.dumps({"format": policy.SESSION_FORMAT, "refused": reason}, indent=2) + "\n",
                    encoding="utf-8",
                )

    # States -----------------------------------------------------------------

    def _capture(self, *, store: bool) -> dict:
        return {
            root_id: capture_tree(Path(root["path"]), self.store if store else None, exclude=self.exclude)
            for root_id, root in self.roots.items()
        }

    def _save_state(self, state: dict) -> str:
        for tree in state.values():
            self._scan_blobs(tree_blobs(tree))
        self.state_counter += 1
        state_id = f"s{self.state_counter}"
        self.states[state_id] = state
        (self.directory / "states").mkdir(exist_ok=True)
        (self.directory / "states" / f"{state_id}.json").write_text(json.dumps(state), encoding="utf-8")
        return state_id

    def _pin(self, state_id: str) -> None:
        if state_id not in self.pinned_states:
            self.pinned_states.append(state_id)

    def _replace_latest_end(self, state_id: str) -> None:
        old = self.latest_end
        self.latest_end = state_id
        if old is None or old in self.pinned_states:
            return
        keep = set(self.pinned_blobs)
        for kept in (*self.pinned_states, state_id):
            for tree in self.states[kept].values():
                keep |= tree_blobs(tree)
        for tree in self.states.pop(old).values():
            for digest in tree_blobs(tree) - keep:
                self.store.delete(digest)
        (self.directory / "states" / f"{old}.json").unlink(missing_ok=True)

    def _fingerprint_matches(self, state_id: str) -> bool:
        recorded = self.states[state_id]
        return all(
            capture_tree(Path(self.roots[root_id]["path"]), exclude=self.exclude) == tree
            for root_id, tree in recorded.items()
        )

    # Spawning -------------------------------------------------------------

    def spawn(self, command, product_args, fd_inputs, *, api: str, **kwargs) -> "RecordingPopen":
        self._join_exited()
        env = dict(os.environ if kwargs.get("env") is None else kwargs["env"])
        cwd = Path(kwargs.get("cwd") or os.getcwd()).resolve()
        with self.lock:
            try:
                event = self._spawn_event(command, product_args, fd_inputs, env, cwd, api, kwargs)
            except Refused as exc:
                self.refuse(str(exc))
                event = None
            proc_id = self.next_proc = self.next_proc + 1
            watch = event.pop("_watch", []) if event is not None else []
            if event is not None:
                event["proc"] = proc_id
                self.emit(event)
                self.steps += 1
                self.write_manifest()
        try:
            process = RecordingPopen(
                self if event is not None and not self.refused else None, proc_id, command, recorded_cwd=cwd, watch=watch, **kwargs
            )
        except BaseException as exc:
            if event is not None:
                self.emit({"event": "spawn_failed", "proc": proc_id, "error": type(exc).__name__})
            raise
        return process

    def _join_exited(self) -> None:
        for process in list(self.live.values()):
            if process.returncode is not None or _has_exited(process.pid):
                process.join_recording(timeout=60)

    def _spawn_event(self, command, product_args, fd_inputs, env, cwd, api, kwargs) -> dict:
        test = _current_test()
        if test and test not in self.tests:
            self.tests.append(test)
        _key, roots = self.recorder._roots(product_args, env, cwd)
        new_roots = {}
        for path, kind in roots:
            if any(policy.inside(path, Path(root["path"])) for root in self.roots.values()):
                continue
            if any(policy.inside(Path(root["path"]), path) for root in self.roots.values()):
                raise Refused("a state root grew around an earlier one mid-session")
            root_id = f"r{len(self.roots)}"
            self.roots[root_id] = {"path": str(path), "kind": kind}
            self.anchors.add(path if kind not in policy.STATE_DIRNAMES else path.parent)
            new_roots[root_id] = path
        cache = env.get("PYTHONPYCACHEPREFIX")
        if cache:
            self.exclude.add(Path(cache).resolve())
        event: dict = {"event": "spawn", "api": api, "test": test, "wall": time.time()}
        if self.wall_start is None:
            self.wall_start = event["wall"]
        if not self.live:
            if not self.started:
                self.started = True
                state_id = self._save_state(self._capture(store=True))
                self._pin(state_id)
                self.start_state = state_id
                self.latest_end = state_id
                event["start_state"] = state_id
            else:
                if not self._fingerprint_matches(self.latest_end):
                    # The test changed the state itself since the last
                    # process exited: a new segment starts from what it left.
                    self._pin(self.latest_end)
                    state_id = self._save_state(self._capture(store=True))
                    self._pin(state_id)
                    segment = {"end": self.latest_end, "start": state_id}
                    self.segments.append(segment)
                    self.latest_end = state_id
                    event["segment"] = segment
                elif new_roots:
                    state_id = self._save_state(self._capture(store=True))
                    self._pin(state_id)
                    event["added_roots"] = {"state": state_id, "roots": sorted(new_roots)}
                    self.latest_end = state_id
        elif new_roots:
            raise Refused("a new state root appeared while a recorded process was live")
        event["after"] = self.after()
        event["args"] = self._args(product_args, fd_inputs)
        event["cwd"] = self._location(cwd)
        event["env"], env_files = self._env(env)
        event["stdio"] = {
            "stdin": _stdio_kind(kwargs.get("stdin")),
            "stdout": _stdio_kind(kwargs.get("stdout")),
            "stderr": _stdio_kind(kwargs.get("stderr")),
            "text": bool(kwargs.get("text") or kwargs.get("universal_newlines") or kwargs.get("encoding") or kwargs.get("errors")),
        }
        if event["stdio"]["stdin"] == "inherit":
            event["stdio"]["stdin_isatty"] = os.isatty(0)
        if kwargs.get("start_new_session"):
            event["start_new_session"] = True
        if "timeout" in kwargs:
            event["timeout"] = kwargs["timeout"]
        inputs = self._inputs([arg for arg in product_args if isinstance(arg, str)], cwd)
        inputs += self._env_file_inputs(env_files)
        event["inputs"] = inputs
        event["_watch"] = sorted({item["path"] for item in inputs if item.get("watch")})
        return event

    def _args(self, product_args, fd_inputs) -> list:
        fd_by_index = {index: data for index, data in fd_inputs}
        recorded = []
        for index, arg in enumerate(product_args):
            if index in fd_by_index:
                recorded.append({"fd_input": self.put(fd_by_index[index]), "synthetic": True})
            else:
                recorded.append(arg)
        return recorded

    def _location(self, path: Path) -> dict:
        if policy.inside(path, ROOT):
            return {"repo": path.relative_to(ROOT).as_posix()}
        if policy.in_temp_area(path):
            self.anchors.add(path)
            return {"path": str(path)}
        raise Refused("working directory outside the repository and the test temp area")

    def _env(self, env: dict) -> tuple[dict, list[tuple[str, Path]]]:
        recorded: dict[str, dict] = {}
        files: list[tuple[str, Path]] = []
        for name in sorted(env):
            value = env[name]
            if name.startswith(policy.TEST_ENV_PREFIX) and name not in policy.HARNESS_ENV:
                self.provenance_env[name] = value
                if name in policy.REPLAYED_TEST_ENV:
                    recorded[name] = {"test": value}
                continue
            if not policy.is_product_env(name):
                continue
            if policy.is_secret_name(name) and value:
                raise Refused(f"secret-named environment variable {name} is set")
            kind = policy.env_path_kind(name, value)
            if kind is None:
                recorded[name] = {"value": value}
                continue
            path = Path(value).resolve()
            if policy.in_temp_area(path):
                recorded[name] = {"path": str(path)}
                if kind == "file":
                    files.append((name, path))
                    self.anchors.add(path.parent)
            elif policy.inside(path, ROOT):
                recorded[name] = {"repo": path.relative_to(ROOT).as_posix()}
            else:
                # A machine path such as the real home: the replay uses its own.
                recorded[name] = {"inherit": True}
        return recorded, files

    def _env_file_inputs(self, files) -> list[dict]:
        items = []
        for name, path in files:
            item = self._input_item(path, env=name)
            if item is not None:
                items.append(item)
        return items

    def _inputs(self, candidates, cwd: Path) -> list[dict]:
        seen: set[str] = set()
        items = []
        for raw in candidates:
            value = raw.split("=", 1)[1] if raw.startswith("--") and "=" in raw else raw
            if not value or "\n" in value or len(value) > 4096:
                continue
            if os.path.isabs(value):
                path = Path(value)
            elif "/" in value and (cwd / value).exists():
                path = cwd / value
            else:
                continue
            path = Path(os.path.abspath(path))
            if str(path) in seen:
                continue
            seen.add(str(path))
            item = self._input_item(path)
            if item is not None:
                items.append(item)
        return items

    def _input_item(self, path: Path, *, env: str | None = None) -> dict | None:
        resolved = path.resolve()
        if any(policy.inside(resolved, Path(root["path"])) for root in self.roots.values()):
            return None
        item: dict = {"path": str(path)}
        if env:
            item["env"] = env
        if policy.in_temp_area(resolved):
            self.anchors.add(path.parent if not path.is_dir() else path)
            item["watch"] = True
            if path.is_file():
                digest = self.put(path.read_bytes())
                if self.produced.get(str(path)) == digest:
                    item["produced"] = True
                else:
                    item["blob"] = digest
            elif path.is_dir():
                item["tree"] = capture_tree(path, self.store, exclude=self.exclude)
                self._scan_blobs(tree_blobs(item["tree"]))
                self.pinned_blobs |= tree_blobs(item["tree"])
            else:
                item["absent"] = True
            return item
        if policy.inside(resolved, ROOT):
            item = {"repo": resolved.relative_to(ROOT).as_posix()}
            if path.is_file():
                item["blob"] = self.put(path.read_bytes())
            elif not path.exists():
                item["absent"] = True
            return item
        if path.exists() or path.is_symlink():
            raise Refused("a command names a path outside the repository and the test temp area")
        return None

    # Exits ----------------------------------------------------------------

    def on_exit(self, process: "RecordingPopen", returncode: int) -> None:
        with self.lock:
            self.live.pop(process.proc_id, None)
            if self.refused:
                return
            try:
                outputs = {}
                for path_text in sorted(process.watched):
                    path = Path(path_text)
                    if path.is_file():
                        digest = self.put(path.read_bytes())
                        outputs[path_text] = {"blob": digest}
                        self.produced[path_text] = digest
                    elif path.is_dir():
                        tree = capture_tree(path, self.store, exclude=self.exclude)
                        self._scan_blobs(tree_blobs(tree))
                        self.pinned_blobs |= tree_blobs(tree)
                        outputs[path_text] = {"tree": tree}
                    else:
                        outputs[path_text] = {"absent": True}
                event = {
                    "event": "exit",
                    "proc": process.proc_id,
                    "returncode": returncode,
                    "outputs": outputs,
                    "wall": time.time(),
                }
                self.wall_end = event["wall"]
                if not self.live:
                    state_id = self._save_state(self._capture(store=True))
                    self._replace_latest_end(state_id)
                    event["state"] = state_id
            except Refused as exc:
                self.refuse(str(exc))
                return
            except OSError as exc:
                # The test removed the root while the process was ending.
                self.incomplete.append(f"proc {process.proc_id}: state unreadable at exit ({type(exc).__name__})")
                event = {"event": "exit", "proc": process.proc_id, "returncode": returncode, "outputs": {}, "wall": time.time()}
            self.emit(event)
            self.events.flush()
            self.write_manifest()

    def write_manifest(self) -> None:
        if self.refused:
            return
        manifest = {
            "format": policy.SESSION_FORMAT,
            "recorder_version": policy.RECORDER_VERSION,
            "comparison_policy": policy.COMPARISON_POLICY,
            "reference": self.recorder.provenance,
            "synthetic": True,
            "tests": self.tests,
            "roots": self.roots,
            "anchors": sorted(str(path) for path in _outermost(self.anchors)),
            "states": {"start": self.start_state, "segments": self.segments, "end": self.latest_end},
            "wall": {"start": self.wall_start, "end": self.wall_end},
            "steps": self.steps,
            "events": self.seq,
            "test_env": self.provenance_env,
            "incomplete": [*self.incomplete, *(f"proc {proc} still running" for proc in sorted(self.live))],
            "notes": self.notes,
        }
        partial = self.directory / ".manifest.json.partial"
        partial.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        os.replace(partial, self.directory / "manifest.json")


def _outermost(paths) -> list[Path]:
    ordered = sorted({Path(path) for path in paths}, key=lambda path: len(path.parts))
    kept: list[Path] = []
    for path in ordered:
        if not any(policy.inside(path, parent) for parent in kept):
            kept.append(path)
    return kept


def _paths_in_text(data: bytes) -> list[str]:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return []
    found = []
    for match in _JSON_STRING.finditer(text):
        try:
            value = json.loads(f'"{match.group(1)}"')
        except json.JSONDecodeError:
            continue
        if os.path.isabs(value):
            found.append(value)
    return found


def _has_exited(pid: int) -> bool:
    try:
        return os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
    except ChildProcessError:
        return True
    except (AttributeError, OSError):
        return False


def _stdio_kind(value) -> str:
    if value is None:
        return "inherit"
    if value is subprocess.PIPE:
        return "pipe"
    if value is subprocess.DEVNULL:
        return "devnull"
    if value is subprocess.STDOUT:
        return "stdout"
    return "file"


class RecordingPopen(subprocess.Popen):
    """A Popen whose pipes run through recording pumps.

    The child gets binary pipes; the test gets fresh OS pipes wrapped the way
    Popen would wrap them, so iteration, `communicate`, `select`, and closing
    behave as without the recorder.
    """

    def __init__(self, session: Session | None, proc_id: int, command, *, recorded_cwd: Path, watch=(), **kwargs):
        self.session = session
        self.proc_id = proc_id
        self.recorded_cwd = str(recorded_cwd)
        self.watched: set[str] = set(watch)
        self._finalizer: threading.Thread | None = None
        stdin = kwargs.pop("stdin", None)
        stdout = kwargs.pop("stdout", None)
        stderr = kwargs.pop("stderr", None)
        text = kwargs.pop("text", None)
        universal_newlines = kwargs.pop("universal_newlines", None)
        encoding = kwargs.pop("encoding", None)
        errors = kwargs.pop("errors", None)
        bufsize = kwargs.pop("bufsize", -1)
        text_mode = bool(encoding or errors or text or universal_newlines)
        # As Popen does: line buffering applies to text streams only.
        line_buffering = text_mode and bufsize == 1
        if bufsize == 1:
            bufsize = -1
        child_stdin = stdin if stdin in (None, subprocess.DEVNULL) else subprocess.PIPE
        child_stderr = subprocess.STDOUT if stderr is subprocess.STDOUT else subprocess.PIPE
        if session is not None:
            with session.lock:
                session.live[proc_id] = self
                session.stdout_lines[proc_id] = 0
        try:
            super().__init__(command, stdin=child_stdin, stdout=subprocess.PIPE, stderr=child_stderr, bufsize=0, **kwargs)
        except BaseException:
            if session is not None:
                with session.lock:
                    session.live.pop(proc_id, None)
            raise
        self.text_mode = text_mode
        self.encoding = encoding
        self.errors = errors
        if session is not None:
            # Process ids appear in lock files the product writes; the replay
            # maps this one to its own child's.
            session.emit({"event": "started", "proc": proc_id, "pid": self.pid})
        child_in, child_out, child_err = self.stdin, self.stdout, self.stderr
        self.stdin = self.stdout = self.stderr = None
        self._out_pumps = []

        if child_in is not None:
            if stdin is subprocess.PIPE:
                read_fd, write_fd = os.pipe()
                handle = io.open(write_fd, "wb", bufsize)
                if text_mode:
                    handle = io.TextIOWrapper(
                        handle, write_through=True, line_buffering=line_buffering, encoding=encoding, errors=errors
                    )
                self.stdin = handle
                source, owned = read_fd, True
            else:
                source, owned = (stdin if isinstance(stdin, int) else stdin.fileno()), False
            threading.Thread(target=self._pump_in, args=(source, owned, child_in), daemon=True).start()
        for name, child_stream, wanted in (("stdout", child_out, stdout), ("stderr", child_err, stderr)):
            if child_stream is None:
                continue
            if wanted is subprocess.PIPE:
                read_fd, write_fd = os.pipe()
                handle = io.open(read_fd, "rb", bufsize)
                if text_mode:
                    handle = io.TextIOWrapper(handle, encoding=encoding, errors=errors)
                setattr(self, name, handle)
                target = write_fd
            elif wanted is subprocess.DEVNULL:
                target = None
            elif wanted is None:
                target = os.dup(1 if name == "stdout" else 2)
            else:
                target = os.dup(wanted if isinstance(wanted, int) else wanted.fileno())
            pump = threading.Thread(target=self._pump_out, args=(name, child_stream, target), daemon=True)
            pump.start()
            self._out_pumps.append(pump)
        self._finalizer = threading.Thread(target=self._finalize, daemon=True)
        self._finalizer.start()

    def _pump_in(self, source: int, owned: bool, child_in) -> None:
        child_fd = child_in.fileno()
        broken = False
        try:
            while True:
                try:
                    data = os.read(source, 65536)
                except OSError:
                    data = b""
                if not data:
                    break
                if self.session is not None:
                    self.session.record_stdin(self.proc_id, data)
                view = memoryview(data)
                try:
                    while view:
                        written = os.write(child_fd, view)
                        view = view[written:]
                except OSError:
                    broken = True
                    break
        finally:
            if self.session is not None:
                self.session.record_stdin(self.proc_id, None, broken=broken)
            try:
                child_in.close()
            except OSError:
                pass
            if owned:
                os.close(source)

    def _pump_out(self, name: str, child_stream, target: int | None) -> None:
        source = child_stream.fileno()
        try:
            while True:
                try:
                    data = os.read(source, 65536)
                except OSError:
                    data = b""
                if not data:
                    break
                if self.session is not None:
                    self.session.record_chunk(self.proc_id, name, data)
                if target is None:
                    continue
                view = memoryview(data)
                try:
                    while view:
                        written = os.write(target, view)
                        view = view[written:]
                except OSError:
                    os.close(target)
                    target = None
        finally:
            child_stream.close()
            if target is not None:
                os.close(target)

    def _finalize(self) -> None:
        for pump in self._out_pumps:
            pump.join()
        returncode = self.wait()
        if self.session is not None:
            self.session.on_exit(self, returncode)

    def join_recording(self, timeout: float | None = None) -> None:
        if self._finalizer is not None and self._finalizer is not threading.current_thread():
            self._finalizer.join(timeout)

    # A test that sees the exit may delete the state roots next; the exit is
    # recorded, roots included, before it can see it.

    def wait(self, timeout=None):
        returncode = super().wait(timeout)
        self.join_recording(timeout=_JOIN_SECONDS)
        return returncode

    def poll(self):
        returncode = super().poll()
        if returncode is not None:
            self.join_recording(timeout=_JOIN_SECONDS)
        return returncode

    def send_signal(self, sig) -> None:
        if self.session is not None and self.returncode is None:
            self.session.record_signal(self.proc_id, sig)
        super().send_signal(sig)


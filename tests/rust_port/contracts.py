"""Deterministic R0 contract snapshots; no book opens or network access.

Regenerate with ``uv run --locked python -m tests.rust_port.contracts update``.
Static indexes record expressions when values cannot be proved from literals;
they deliberately do not guess that a dynamic retry policy is false.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import tempfile
from pathlib import Path

from tests import cli_runner


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests/fixtures/rust_port"


def document(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode()


def sources():
    for path in sorted((ROOT / "kassiber").rglob("*.py")):
        yield path.relative_to(ROOT), ast.parse(path.read_text(encoding="utf-8"))


def constants(tree: ast.AST) -> dict[str, object]:
    result = {}
    for node in getattr(tree, "body", ()):
        if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    result[target.id] = node.value.value
        elif (
            isinstance(node, ast.AnnAssign)
            and isinstance(node.target, ast.Name)
            and isinstance(node.value, ast.Constant)
        ):
            result[node.target.id] = node.value.value
    return result


def value(node: ast.AST | None, names: dict[str, object]):
    if node is None:
        return None
    if isinstance(node, ast.Constant):
        return node.value
    if (
        isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub))
        and isinstance(node.operand, ast.Constant)
    ):
        return ast.literal_eval(node)
    if isinstance(node, ast.Name) and node.id in names:
        return names[node.id]
    return {"expression": ast.unparse(node)}


def commands_describe() -> bytes:
    # Use the actual CLI and preserve its output bytes. The audit hook forbids
    # even a loopback socket, independently of product consent switches.
    with tempfile.TemporaryDirectory(prefix="kassiber-r0-catalog-") as directory:
        root = Path(directory)
        (root / "sitecustomize.py").write_text(
            "import sys\n"
            "def deny(event, args):\n"
            "    if event.startswith('socket.'):\n"
            "        raise RuntimeError('R0 catalog must not use sockets')\n"
            "sys.addaudithook(deny)\n",
            encoding="utf-8",
        )
        env = {key: os.environ[key] for key in ("PATH", "SYSTEMROOT", "WINDIR", "HOME") if key in os.environ}
        env.update({
            "USERPROFILE": directory, "TMPDIR": directory,
            "LOCALAPPDATA": directory, "XDG_DATA_HOME": directory,
            "XDG_CONFIG_HOME": directory, "XDG_STATE_HOME": directory,
            "PYTHONPATH": directory, "PYTHONIOENCODING": "utf-8",
            "KASSIBER_NO_EGRESS": "1", "KASSIBER_DISABLE_UPDATE_CHECK": "1",
            "KASSIBER_OFFLINE_PREFERENCE_FILE": str(root / "offline.json"),
            "KASSIBER_AGENT_ACCESS_FILE": str(root / "agents.json"),
            "KASSIBER_UPDATE_PREFERENCE_FILE": str(root / "updates.json"),
            "KASSIBER_UPDATE_CACHE_FILE": str(root / "cache.json"),
        })
        # The catalog pins the Python reference, never a configured candidate.
        result = cli_runner.run(
            ["--machine", "--data-root", str(root / "data"), "commands", "describe"], reference=True,
            cwd=ROOT, env=env, capture_output=True, check=True, timeout=60,
            own_egress_guard="an audit hook that denies every socket, loopback included",
        )
        assert not result.stderr, result.stderr.decode("utf-8", errors="replace")
        assert json.loads(result.stdout)["kind"] == "commands.describe"
        return result.stdout


def declared_strings(path: str, name: str) -> list[str]:
    source = (ROOT / path).read_text(encoding="utf-8")
    match = re.search(r"const " + name + r"\b[^=]*=\s*(?:new Set\(|&)?\[(.*?)\]", source, re.S)
    if match is None:
        raise ValueError(f"missing declaration {path}:{name}")
    # Strip comments so a quoted example in a comment cannot become a kind.
    body = re.sub(r"//[^\n]*|/\*.*?\*/", "", match[1], flags=re.S)
    return sorted(re.findall(r'"([a-z][a-z0-9_.-]*)"', body))


def daemon_kinds() -> dict:
    from kassiber.daemon import SUPPORTED_KINDS

    rust = "ui-tauri/src-tauri/src/lib.rs"
    vite = "ui-tauri/vite.config.ts"
    lists = {
        "tauri_allowed": declared_strings(rust, "ALLOWED_DAEMON_KINDS"),
        "tauri_streaming": declared_strings(rust, "STREAMING_DAEMON_KINDS"),
        "tauri_ai_runtime": declared_strings(rust, "AI_RUNTIME_KINDS"),
        "vite_allowed": declared_strings(vite, "ALLOWED_BRIDGE_KINDS"),
        "vite_streaming": declared_strings(vite, "STREAM_CAPABLE_BRIDGE_KINDS"),
        "vite_stream_only": declared_strings(vite, "STREAM_ONLY_BRIDGE_KINDS"),
    }
    emissions = []
    for path, tree in sources():
        if not path.name.startswith("daemon"):
            continue
        names = constants(tree)
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            func = ast.unparse(node.func).split(".")[-1]
            offset = 1 if func == "_emit_background_freshness_event" else 0
            builders = {"build_envelope", "build_event_envelope", "_emit_background_freshness_event"}
            if func not in builders or len(node.args) <= offset:
                continue
            kind = value(node.args[offset], names)
            classification = (
                "event" if func != "build_envelope" else
                "startup" if kind == "daemon.ready" else
                "terminal" if isinstance(kind, str) and kind in SUPPORTED_KINDS else
                "control" if isinstance(kind, str) and "secret_store" in kind else
                "stream" if isinstance(kind, str) else "dynamic"
            )
            emissions.append({"at": f"{path}:{node.lineno}", "kind": kind, "classification": classification})
    return {
        "schema_version": 1,
        "terminal_rule": (
            "Matching request_id and exact request kind, or error; "
            "event:true has no request_id; daemon.ready is the startup exception"
        ),
        "requests": [
            {"kind": kind, "terminal_kind": kind, **{key: kind in values for key, values in lists.items()}}
            for kind in sorted(SUPPORTED_KINDS)
        ],
        "allowlists": lists,
        "emission_sites": sorted(emissions, key=lambda item: (item["at"].split(":")[0], int(item["at"].split(":")[1]))),
    }


def error_codes() -> dict:
    sites = []
    defaults = []
    bindings = []
    modules = list(sources())
    by_module = {".".join(path.with_suffix("").parts): (path, tree) for path, tree in modules}
    for path, tree in modules:
        names = constants(tree)
        functions = {
            node.name: node for node in ast.walk(tree)
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        classes = {node.name: node for node in ast.walk(tree) if isinstance(node, ast.ClassDef)}
        imported = {}
        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom):
                package = list(path.with_suffix("").parts[:-1])
                base = package[:len(package) - node.level + 1] if node.level else []
                module = ".".join([*base, *([node.module] if node.module else [])])
                if module in by_module:
                    target_path, target_tree = by_module[module]
                    for alias in node.names:
                        target = next((
                            fn for fn in target_tree.body
                            if isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef))
                            and fn.name == alias.name
                        ), None)
                        if target is not None:
                            imported[alias.asname or alias.name] = (target_path, target)

        def argument(call, function, parameter):
            """Resolve positional arguments and wrapper defaults, not just kwargs."""
            keywords = {kw.arg: kw.value for kw in call.keywords}
            if parameter in keywords:
                return keywords[parameter]
            if function is None:
                return None
            args = function.args.posonlyargs + function.args.args
            for position, arg in enumerate(args):
                if arg.arg == parameter:
                    if position < len(call.args):
                        return call.args[position]
                    offset = len(args) - len(function.args.defaults)
                    if position >= offset:
                        return function.args.defaults[position - offset]
            for arg, default in zip(function.args.kwonlyargs, function.args.kw_defaults):
                if arg.arg == parameter:
                    return default
            return None

        for node in ast.walk(tree):
            if isinstance(node, (ast.Assign, ast.AnnAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                for target in targets:
                    name = ast.unparse(target)
                    if name.lower().endswith(("code", "codes")) and node.value is not None:
                        bindings.append({"at": f"{path}:{node.lineno}", "name": name, "value": value(node.value, names)})
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                args = node.args.posonlyargs + node.args.args
                for arg, default in zip(args[-len(node.args.defaults):], node.args.defaults):
                    if arg.arg == "code":
                        defaults.append({"at": f"{path}:{node.lineno}", "function": node.name, "code": value(default, names)})
                for arg, default in zip(node.args.kwonlyargs, node.args.kw_defaults):
                    if arg.arg == "code" and default is not None:
                        defaults.append({"at": f"{path}:{node.lineno}", "function": node.name, "code": value(default, names)})
            if isinstance(node, ast.Call):
                callee = ast.unparse(node.func)
                leaf = callee.split(".")[-1]
                function = functions.get(callee)
                function_path = path
                if function is None and callee in imported:
                    function_path, function = imported[callee]
                code = argument(node, function, "code")
                if code is None and callee.split(".")[-1] == "build_error_envelope" and node.args:
                    code = node.args[0]
                if code is None and callee.split(".")[-1] == "AppError":
                    code = node.args[1] if len(node.args) > 1 else ast.Constant("app_error")
                if code is None:
                    continue
                retry = argument(node, function, "retryable")
                if retry is None and callee.split(".")[-1] == "build_error_envelope" and len(node.args) > 4:
                    retry = node.args[4]
                if retry is None and leaf == "AppError" and len(node.args) > 4:
                    retry = node.args[4]
                retryable = value(retry, names) if retry is not None else None
                if retryable is None and leaf in {"AppError", "build_error_envelope"}:
                    retryable = False
                if retryable is None and callee == "super().__init__":
                    owner = next((cls for cls in classes.values() if cls.lineno <= node.lineno <= cls.end_lineno), None)
                    if owner is not None and any(
                        ast.unparse(base).split(".")[-1] == "AppError" for base in owner.bases
                    ):
                        retryable = False
                if retryable is None and function is not None:
                    # A wrapper's body may explicitly fix or compute the flag.
                    policies = []
                    for inner in ast.walk(function):
                        if isinstance(inner, ast.Call) and ast.unparse(inner.func).split(".")[-1] in {
                            "AppError", "build_error_envelope",
                        }:
                            policy = next((
                                kw.value for kw in inner.keywords if kw.arg == "retryable"
                            ), ast.Constant(False))
                            # Common wrappers compare their literal code; retain
                            # a dynamic expression only if it truly needs context.
                            if (
                                isinstance(policy, ast.Compare)
                                and isinstance(policy.left, ast.Name) and policy.left.id == "code"
                                and len(policy.ops) == 1 and isinstance(policy.ops[0], ast.Eq)
                                and isinstance(policy.comparators[0], ast.Constant)
                                and isinstance(value(code, names), str)
                            ):
                                policies.append(value(code, names) == policy.comparators[0].value)
                            else:
                                policies.append(value(policy, names))
                    if policies and all(policy == policies[0] for policy in policies):
                        retryable = policies[0]
                    elif not policies:
                        retryable = None
                if retryable is None:
                    # A known code-bearing record/helper is not an exception.
                    retryable = None if function is not None or callee in classes else {
                        "expression": "callee policy; inspect wrapper"
                    }
                site = {
                    "at": f"{path}:{node.lineno}", "form": "call", "callee": callee,
                    "code": value(code, names), "retryable": retryable,
                }
                if function is not None:
                    site["policy_at"] = f"{function_path}:{function.lineno}"
                sites.append(site)
            elif isinstance(node, ast.Dict):
                pairs = {
                    key.value: val for key, val in zip(node.keys, node.values)
                    if isinstance(key, ast.Constant) and isinstance(key.value, str)
                }
                if "code" in pairs:
                    sites.append({
                        "at": f"{path}:{node.lineno}", "form": "record",
                        "code": value(pairs["code"], names),
                        "retryable": value(pairs["retryable"], names) if "retryable" in pairs else None,
                    })
    return {
        "schema_version": 1,
        "note": (
            "Code keyword/record sites plus positional wrappers, code variable bindings and defaults, "
            "including non-error issue codes; null retryable means no declared retry policy on a "
            "record/helper, expressions require runtime context. Defaults and bindings are indexed separately."
        ),
        "defaults": defaults, "bindings": bindings,
        "sites": sorted(sites, key=lambda row: (
            row["at"].split(":")[0], int(row["at"].split(":")[1]), row["form"],
        )),
    }


def environment_reads() -> dict:
    # Scan literals, not just direct getenv calls: families are assembled and
    # dotenv values share spellings. Include the frozen launcher explicitly.
    paths = [path for path, _ in sources()] + [Path("scripts/kassiber_pyinstaller_entry.py")]
    tokens = (
        r"\b(?:KASSIBER_[A-Z0-9_]+|SATBOOKS_[A-Z0-9_]+|XDG_[A-Z0-9_]+|HOME|USERPROFILE|"
        r"HOMEDRIVE|HOMEPATH|LOCALAPPDATA|APPIMAGE|APPDIR|SSL_CERT_FILE|SSL_CERT_DIR|"
        r"TERM|NO_COLOR|CLICOLOR|PYINSTALLER_RESET_ENVIRONMENT)\b"
    )
    found: dict[str, list[str]] = {}
    for path in paths:
        for line, text in enumerate((ROOT / path).read_text(encoding="utf-8").splitlines(), 1):
            for name in sorted(set(re.findall(tokens, text))):
                found.setdefault(name, []).append(f"{path}:{line}")
    reads = []
    for path in paths:
        tree = ast.parse((ROOT / path).read_text(encoding="utf-8"))
        names = constants(tree)
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and node.args and ast.unparse(node.func) in {
                "os.getenv", "os.environ.get", "environ.get", "environment.get",
            }:
                reads.append({
                    "at": f"{path}:{node.lineno}", "name": value(node.args[0], names),
                    "default": value(node.args[1], names) if len(node.args) > 1 else None,
                })
            elif isinstance(node, ast.Subscript) and ast.unparse(node.value) == "os.environ" and isinstance(node.ctx, ast.Load):
                reads.append({"at": f"{path}:{node.lineno}", "name": value(node.slice, names), "default": None})
    return {
        "schema_version": 1,
        "note": (
            "Literal occurrences include docs/comments and writes; reads separately index actual "
            "environment access. The reference document explains precedence, dynamic families "
            "and implicit library inputs."
        ),
        "variables": dict(sorted(found.items())), "reads": reads,
    }


SNAPSHOT_NAMES = ("commands_describe.json", "daemon_kinds.json", "error_codes.json", "environment.json")


def snapshot(name: str) -> bytes:
    if name == "commands_describe.json":
        return commands_describe()
    if name == "daemon_kinds.json":
        return document(daemon_kinds())
    if name == "error_codes.json":
        return document(error_codes())
    if name == "environment.json":
        return document(environment_reads())
    raise KeyError(name)


def snapshots() -> dict[str, bytes]:
    return {name: snapshot(name) for name in SNAPSHOT_NAMES}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("update", "check"))
    args = parser.parse_args()
    bad = []
    for name, content in snapshots().items():
        path = FIXTURES / name
        if args.action == "update":
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        elif not path.exists() or path.read_bytes() != content:
            bad.append(name)
    if bad:
        print("R0 contract drift: " + ", ".join(bad) + "; review, then run contracts update")
    return bool(bad)


if __name__ == "__main__":
    raise SystemExit(main())

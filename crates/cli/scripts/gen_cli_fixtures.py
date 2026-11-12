"""Write tests/data/argparse_cases.txt: Python's argparse answers.

For the Rust CLI's parser tests, from `build_parser()` at this checkout:

- `usage`: every parser's `format_usage()` at COLUMNS 80, 40, and 200;
- `help`: every parser's `format_help()` at COLUMNS 80;
- `kind`: every leaf's `derive_kind` and `command_path`;
- `parse`: an argv and its outcome: the namespace, the error (the parser
  that raised it, its message and usage), or the text `-h`/`--version`
  printed. Crafted cases plus seeded random command lines over the tree.
  An error's usage is its parser's `format_usage()`, pinned by `usage`.

The parsers are numbered as `gen_parser_spec.py` numbers them.

    .venv/bin/python crates/cli/scripts/gen_cli_fixtures.py
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = ROOT / "crates" / "cli" / "tests" / "data" / "argparse_cases.txt"
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

VERSION = "Kassiber 0.22.78"
SEED = 21


class Raised(Exception):
    def __init__(self, parser, message):
        super().__init__(message)
        self.parser = parser
        self.message = message


class Exited(Exception):
    def __init__(self, stdout):
        super().__init__(stdout)
        self.stdout = stdout


def set_columns(columns: int) -> None:
    os.environ["COLUMNS"] = str(columns)
    os.environ["LINES"] = "24"


def build():
    import importlib

    cli_main = importlib.import_module("kassiber.cli.main")

    cli_main.version_text = lambda: VERSION
    parser = cli_main.build_parser()
    from gen_parser_spec import collect

    parsers = collect(parser)
    index = {id(p): i for i, p in enumerate(parsers)}

    def install(current):
        def fail(message, _parser=current):
            raise Raised(_parser, message)

        def exit_(status=0, message=None, _parser=current):
            raise Exited(None)

        current.error = fail
        current.exit = exit_

    for current in parsers:
        install(current)
    return parser, parsers, index


def outcome(parser, index, argv):
    stdout = io.StringIO()
    try:
        with contextlib.redirect_stdout(stdout):
            namespace = parser.parse_args(argv)
    except Raised as raised:
        # The usage ops pin each parser's usage text; an error names the
        # parser whose usage it carries.
        return {"error": {"parser": index[id(raised.parser)], "message": raised.message}}
    except Exited:
        return {"exit": stdout.getvalue()}
    return {"ok": json.dumps(vars(namespace), sort_keys=True, separators=(",", ":"))}


def leaves(parser, path=()):
    sub = next((a for a in parser._actions if isinstance(a, argparse._SubParsersAction)), None)
    if sub is None:
        yield path, parser
        return
    for name, child in sub.choices.items():
        yield from leaves(child, (*path, name))


GLOBALS = [
    ["--machine"], ["--format", "json"], ["--format=csv"], ["--format", "xml"], ["--fo", "plain"],
    ["--mach"], ["--non-interactive"], ["--debug"], ["--project", "p1"], ["--data-root", "/tmp/r"],
    ["--data-root=/tmp/r"], ["--env-file", "x.env"], ["--output", "-"], ["--output=out.json"],
    ["--diagnostics-out", "auto"], ["--db-passphrase-fd", "3"], ["--db-passphrase-fd", "x"],
    ["--db-passphrase-fd", "٣"], ["--operator-auth-fd=4"], ["--operator-auth-fd"], ["--d"],
    ["--de"], ["--di", "auto"], ["--db", "3"], ["--n"], ["--o", "f"], ["--op", "4"],
]

CRAFTED = [
    [], ["--version"], ["-h"], ["--help"], ["--he"], ["--machine"], ["--machine", "--version"],
    ["--version", "bogus"], ["bogus"], ["--machine", "bogus"], ["status"], ["status", "--machine"],
    ["--machine", "status", "extra"], ["status", "-h"], ["status", "--h"], ["daemon"],
    ["commands", "describe"], ["commands", "describe", "wallets", "sync"],
    ["commands", "describe", "--", "--x"], ["commands", "describe", "-x"], ["commands"],
    ["commands", "bogus"], ["--machine", "commands", "describe", "status"], ["--format"],
    ["--format", "json", "--format", "csv", "status"], ["--machine=1", "status"], ["--machine=", "status"],
    ["-x", "status"], ["-hx"], ["-h=x"], ["--", "status"], ["status", "--"], ["-", "status"],
    ["--data-root", "--machine", "status"], ["--data-root", "-5", "status"], ["--data-root", "-.5", "status"],
    ["--data-root", "has space", "status"], ["--project", "-x y", "status"], ["--format", "", "status"],
    ["wallets"], ["wallets", "list"], ["wallets", "sync", "--all"], ["wallets", "bogus"],
    ["wallets", "import-bull", "--help"], ["wallets", "import-bullbitcoin", "-h"],
    ["update", "--enable-checks", "--disable-checks"], ["update", "--status", "--status"],
    ["attachments", "add"], ["attachments", "add", "--file", "f", "--url", "u"],
    ["review", "receipt"], ["reports", "psbt-privacy"],
    ["reports", "lightning-profitability", "--window-days", "0"],
    ["reports", "lightning-profitability", "--window-days", "x"],
    ["reports", "lightning-profitability", "--window-days", "365"],
    ["transfers", "components", "plan", "--draft", "--activate"],
    ["ai", "providers", "update", "x", "--enable", "--disable"],
    ["metadata", "records", "tag", "add"], ["projects", "list"], ["projects"],
    ["--machine", "wallets", "list", "--machine"], ["--machine", "--machine", "status"],
    ["status", "--format", "json"], ["--operator-broker-server"], ["--migrate-default-state-root"],
    ["--project", "a", "--data-root", "b", "status"], ["--output"], ["--output", "--machine"],
]


def random_value(rng, action):
    if action.choices is not None and not isinstance(action, argparse._SubParsersAction):
        choices = list(action.choices)
        return rng.choice(choices + ["bogus"]) if rng.random() < 0.9 else rng.choice(choices).upper()
    if action.type is int:
        return rng.choice(["3", "-1", "x", "1_0", "٣", "", "99999999999999999999"])
    if action.type is float:
        return rng.choice(["1.5", "1e3", "nan", "x", "-2", ""])
    if getattr(action.type, "__name__", "") == "_lightning_window_days":
        return rng.choice(["1", "365", "366", "0", "x"])
    return rng.choice(["v", "a b", "-5", "--x", "", "café", "=", "x=y"])


def mutate_option(rng, option):
    if option.startswith("--") and len(option) > 4 and rng.random() < 0.2:
        return option[: rng.randrange(3, len(option))]
    return option


def random_argv(rng, parser, leaf_list):
    path, leaf = rng.choice(leaf_list)
    argv = []
    for _ in range(rng.randrange(0, 3)):
        argv.extend(rng.choice(GLOBALS))
    argv.extend(path)
    if rng.random() < 0.05 and path:
        argv[-1] = argv[-1] + "x"
    actions = [a for a in leaf._actions if not isinstance(a, argparse._HelpAction)]
    if rng.random() < 0.02:
        actions.append(leaf._actions[0])
    for action in rng.sample(actions, k=min(len(actions), rng.randrange(0, 5))):
        if action.option_strings:
            option = mutate_option(rng, rng.choice(action.option_strings))
            if action.nargs == 0:
                argv.append(option + ("=1" if rng.random() < 0.05 else ""))
            elif rng.random() < 0.15:
                argv.append(f"{option}={random_value(rng, action)}")
            elif rng.random() < 0.05:
                argv.append(option)
            else:
                argv.extend([option, random_value(rng, action)])
        else:
            count = 1 if action.nargs in (None, "?") else rng.randrange(0, 3)
            argv.extend(random_value(rng, action) for _ in range(count))
    if rng.random() < 0.1:
        argv.insert(rng.randrange(0, len(argv) + 1), rng.choice(["--", "--machine", "-h", "--bogus", "x"]))
    if rng.random() < 0.05 and argv:
        argv.pop(rng.randrange(len(argv)))
    return argv


def main() -> int:
    if sys.version_info[:2] != (3, 11):
        raise SystemExit("run with the locked Python 3.11 interpreter")
    rng = random.Random(SEED)
    set_columns(80)
    parser, parsers, index = build()
    lines = []
    for columns in (80, 40, 200):
        set_columns(columns)
        for i, current in enumerate(parsers):
            lines.append({"op": "usage", "parser": i, "width": columns - 2, "text": current.format_usage()})
    set_columns(80)
    for i, current in enumerate(parsers):
        lines.append({"op": "help", "parser": i, "width": 78, "text": current.format_help()})
    from kassiber.cli.command_registry import command_path
    from kassiber.envelope import derive_kind

    leaf_list = list(leaves(parser))
    for path, leaf in leaf_list:
        dests = {}
        current = parser
        for name in path:
            sub = next(a for a in current._actions if isinstance(a, argparse._SubParsersAction))
            dests[sub.dest] = name
            current = sub.choices[name]
        namespace = argparse.Namespace(**dests)
        lines.append({
            "op": "kind", "path": list(path), "dests": dests,
            "kind": derive_kind(namespace), "command_path": command_path(namespace),
        })
    for argv in CRAFTED:
        lines.append({"op": "parse", "argv": argv, "out": outcome(parser, index, argv)})
    for columns in (40, 200):
        set_columns(columns)
        for argv in (["bogus"], ["wallets", "bogus"], ["status", "--x"], ["-h"], ["wallets", "-h"]):
            lines.append({"op": "parse", "argv": argv, "width": columns - 2,
                          "out": outcome(parser, index, argv)})
    set_columns(80)
    for _ in range(5000):
        argv = random_argv(rng, parser, leaf_list)
        lines.append({"op": "parse", "argv": argv, "out": outcome(parser, index, argv)})
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(
        "\n".join(json.dumps(line, separators=(",", ":"), ensure_ascii=True) for line in lines) + "\n",
        encoding="ascii",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

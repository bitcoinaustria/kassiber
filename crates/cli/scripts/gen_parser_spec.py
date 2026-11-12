"""Generate the Rust CLI's argparse tree from kassiber/cli/main.py.

`build_parser()` is the Python CLI's grammar. This script walks every parser
it builds (380 today, one per command group and leaf) and writes their
actions, option strings, mutually exclusive groups, and help sections as
Rust statics in `crates/cli/src/spec_data.rs`, so the Rust parser accepts
and rejects exactly what argparse does and prints the same usage. The
Rust-only leaves (`RUST_ONLY_LEAVES`) are grafted onto that tree first. The
behaviour of each leaf is ported separately; this file only carries its
grammar.

    .venv/bin/python crates/cli/scripts/gen_parser_spec.py           # write
    .venv/bin/python crates/cli/scripts/gen_parser_spec.py --check   # drift

`--check` fails when kassiber/cli/main.py changed the grammar without the
Rust tree following.
"""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = ROOT / "crates" / "cli" / "src" / "spec_data.rs"
sys.path.insert(0, str(ROOT))

KINDS = {
    argparse._StoreAction: "Store",
    argparse._StoreConstAction: "StoreConst",
    argparse._StoreTrueAction: "StoreTrue",
    argparse._StoreFalseAction: "StoreFalse",
    argparse._AppendAction: "Append",
    argparse.BooleanOptionalAction: "BooleanOptional",
    argparse._HelpAction: "Help",
    argparse._VersionAction: "Version",
    argparse._SubParsersAction: "SubParsers",
}
NARGS = {
    None: "Nargs::Single",
    argparse.OPTIONAL: "Nargs::Optional",
    argparse.ZERO_OR_MORE: "Nargs::ZeroOrMore",
    argparse.ONE_OR_MORE: "Nargs::OneOrMore",
    argparse.PARSER: "Nargs::Parser",
}


def rust_str(text: str) -> str:
    out = ['"']
    for char in text:
        point = ord(char)
        if char == "\\":
            out.append("\\\\")
        elif char == '"':
            out.append('\\"')
        elif char == "\n":
            out.append("\\n")
        elif char == "\t":
            out.append("\\t")
        elif char == "\r":
            out.append("\\r")
        elif 0x20 <= point < 0x7F:
            out.append(char)
        else:
            out.append(f"\\u{{{point:x}}}")
    out.append('"')
    return "".join(out)


def lit(value) -> str:
    if value is argparse.SUPPRESS:
        return "Lit::Suppress"
    if value is None:
        return "Lit::None"
    if isinstance(value, bool):
        return f"Lit::Bool({str(value).lower()})"
    if isinstance(value, int):
        return f"Lit::Int({value})"
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError(f"non-finite float literal {value!r}")
        return f"Lit::Float({value!r}_f64)"
    if isinstance(value, str):
        return f"Lit::Str({rust_str(value)})"
    if isinstance(value, (list, tuple)):
        return "Lit::List(&[" + ", ".join(lit(item) for item in value) + "])"
    raise ValueError(f"unsupported literal {value!r}")


def value_type(action: argparse.Action) -> str:
    if action.type is None:
        return "ValueType::Str"
    if action.type is int:
        return "ValueType::Int"
    if action.type is float:
        return "ValueType::Float"
    name = getattr(action.type, "__name__", "")
    if name == "_lightning_window_days":
        return "ValueType::LightningWindowDays"
    raise ValueError(f"unsupported argument type {action.type!r} for {action.dest}")


def help_value(text) -> str:
    if text is argparse.SUPPRESS:
        return "Help::Suppressed"
    if text is None:
        return "Help::Missing"
    if "%" in text:
        raise ValueError(f"help with % expansion is not supported: {text!r}")
    if text == TAX_COUNTRY_HELP:
        # The Rust registry lists its own countries (crates/tax-countries).
        return "Help::TaxCountries"
    if text == POOL_SCOPE_HELP:
        return "Help::PoolScopes"
    if text in AI_PROVIDER_KIND_HELP:
        # The Rust-only `cli_proxy` kind (docs/reference/rust-shell.md#cli-proxy-providers).
        return AI_PROVIDER_KIND_HELP[text]
    return f"Help::Text({rust_str(text)})"


TAX_COUNTRY_HELP = "Tax country for the book (CLI profile; currently supported: generic, at)"
POOL_SCOPE_HELP = "Country-policy-controlled cost-basis pool scope (currently: global)."
AI_PROVIDER_KIND_HELP = {
    "local = on-machine, remote = data leaves the device, tee = encrypted attestation provider. "
    "Default: local for a loopback URL, else remote (tee is never inferred; remote still needs --acknowledge)":
    "Help::AiProviderKind",
    "Change the privacy kind. Unchanged by default, except that a local provider given a non-loopback "
    "--base-url becomes remote; leaving local always needs --acknowledge again":
    "Help::AiProviderKindChange",
}
# Choices only the Rust backend takes, appended to Python's: a CLI proxy
# provider (docs/reference/rust-shell.md#cli-proxy-providers).
RUST_ONLY_CHOICES = {
    ("kassiber ai providers create", "kind"): ("cli_proxy",),
    ("kassiber ai providers update", "kind"): ("cli_proxy",),
}


def rust_default(action: argparse.Action):
    """Python's default, except where the Rust registry's country decides it:
    an absent `--cost-basis-pool-scope` takes the country's default scope
    (`global` for generic and Austrian books, as Python's)."""
    if action.dest == "cost_basis_pool_scope":
        return None
    return action.default


#: Leaves only the Rust backend serves, as dotted command paths. Python's
#: grammar lacks them, so Python refuses them as an invalid choice
#: (docs/reference/rust-shell.md#rust-only-commands). Keep in step with
#: `crates/cli/src/spec.rs::RUST_ONLY_LEAVES`.
RUST_ONLY_LEAVES = ("wallets.sync-bitpanda",)
#: Backend kinds only the Rust backend accepts (`backends create|update
#: --kind`). Keep in step with `crates/cli/src/spec.rs::RUST_ONLY_BACKEND_KINDS`.
RUST_ONLY_BACKEND_KINDS = ("bitpanda",)


def add_rust_only(parser: argparse.ArgumentParser) -> None:
    """Graft the Rust-only leaves and backend kinds onto Python's tree.
    `wallets sync-bitpanda` takes the arguments of the other exchange syncs
    and follows them in the choice list; `--kind` choices stay sorted."""

    def subparsers(current: argparse.ArgumentParser) -> argparse._SubParsersAction:
        return next(a for a in current._actions if isinstance(a, argparse._SubParsersAction))

    backends = subparsers(subparsers(parser).choices["backends"])
    for leaf in ("create", "update"):
        kind = next(a for a in backends.choices[leaf]._actions if a.dest == "kind")
        kind.choices = sorted({*kind.choices, *RUST_ONLY_BACKEND_KINDS})

    wallets = subparsers(subparsers(parser).choices["wallets"])
    child = wallets.add_parser("sync-bitpanda")
    child.add_argument("--workspace")
    child.add_argument("--profile")
    child.add_argument("--backend", required=True)
    child.add_argument("--wallet")
    names = list(wallets.choices.items())
    names.remove(("sync-bitpanda", child))
    at = next(i for i, (name, _) in enumerate(names) if name == "sync-binance") + 1
    names.insert(at, ("sync-bitpanda", child))
    wallets.choices.clear()
    wallets.choices.update(names)


def collect(parser: argparse.ArgumentParser) -> list[argparse.ArgumentParser]:
    """Every distinct parser, depth first, aliases visited once."""

    ordered: list[argparse.ArgumentParser] = []
    seen: set[int] = set()

    def walk(current: argparse.ArgumentParser) -> None:
        if id(current) in seen:
            return
        seen.add(id(current))
        ordered.append(current)
        for action in current._actions:
            if isinstance(action, argparse._SubParsersAction):
                for child in action.choices.values():
                    walk(child)

    walk(parser)
    return ordered


def check_parser(parser: argparse.ArgumentParser) -> None:
    assert parser.prefix_chars == "-", parser.prog
    assert parser.allow_abbrev, parser.prog
    assert parser.fromfile_prefix_chars is None, parser.prog
    assert parser.usage is None, parser.prog
    assert parser.epilog is None, parser.prog
    assert parser.exit_on_error, parser.prog
    assert parser.argument_default is None, parser.prog
    assert type(parser.formatter_class) is type and parser.formatter_class is argparse.HelpFormatter
    assert not parser._has_negative_number_optionals, parser.prog
    for action in parser._actions:
        assert type(action) in KINDS, (parser.prog, type(action))
        assert action.nargs in NARGS or action.nargs == 0, (parser.prog, action.nargs)
        assert action.metavar is None or isinstance(action.metavar, str), parser.prog
        if isinstance(action.default, str) and action.type is not None:
            raise ValueError(f"{parser.prog}: str default for typed {action.dest}")
        if action.choices is not None and not isinstance(action, argparse._SubParsersAction):
            assert all(isinstance(choice, str) for choice in action.choices), parser.prog
        if isinstance(action, argparse._VersionAction):
            assert parser.prog == "kassiber", parser.prog


def render_action(action: argparse.Action, parsers: dict[int, int], prog: str) -> str:
    kind = KINDS[type(action)]
    fields = [
        f"kind: ActionKind::{kind}",
        "option_strings: &[" + ", ".join(rust_str(o) for o in action.option_strings) + "]",
        "dest: " + ("None" if action.dest is argparse.SUPPRESS else f"Some({rust_str(action.dest)})"),
        "nargs: " + ("Nargs::Zero" if action.nargs == 0 else NARGS[action.nargs]),
        f"constant: {lit(action.const)}",
        f"default: {lit(rust_default(action))}",
        f"value_type: {value_type(action)}",
        "required: " + ("true" if action.required else "false"),
        f"help: {help_value(action.help)}",
        "metavar: " + ("None" if action.metavar is None else f"Some({rust_str(action.metavar)})"),
    ]
    if isinstance(action, argparse._SubParsersAction):
        names = ", ".join(
            f"({rust_str(name)}, {parsers[id(child)]})" for name, child in action.choices.items()
        )
        pseudo = ", ".join(
            "ChoiceHelp { "
            f"dest: {rust_str(choice.dest)}, metavar: {rust_str(choice.metavar)}, "
            f"help: {help_value(choice.help)} }}"
            for choice in action._choices_actions
        )
        fields.append("choices: None")
        fields.append(f"subparsers: Some(&SubParsers {{ names: &[{names}], choice_help: &[{pseudo}] }})")
    else:
        if action.choices is None:
            fields.append("choices: None")
        else:
            choices = [*action.choices, *RUST_ONLY_CHOICES.get((prog, action.dest), ())]
            fields.append("choices: Some(&[" + ", ".join(rust_str(c) for c in choices) + "])")
        fields.append("subparsers: None")
    return "Action { " + ", ".join(fields) + " }"


def render(parser: argparse.ArgumentParser) -> str:
    # Rust-only parsers come last, so every Python parser keeps the index
    # Python's fixtures (`gen_cli_fixtures.py`) record for it.
    rust_only = {"kassiber " + leaf.replace(".", " ") for leaf in RUST_ONLY_LEAVES}
    parsers = collect(parser)
    parsers = [p for p in parsers if p.prog not in rust_only] + [p for p in parsers if p.prog in rust_only]
    index = {id(p): i for i, p in enumerate(parsers)}
    blocks = []
    for current in parsers:
        check_parser(current)
        position = {id(action): i for i, action in enumerate(current._actions)}
        actions = ",\n        ".join(render_action(a, index, current.prog) for a in current._actions)
        options = ", ".join(
            f"({rust_str(option)}, {position[id(action)]})"
            for option, action in current._option_string_actions.items()
        )
        groups = ", ".join(
            "Mutex { required: " + ("true" if group.required else "false") + ", actions: &["
            + ", ".join(str(position[id(a)]) for a in group._group_actions) + "] }"
            for group in current._mutually_exclusive_groups
        )
        sections = ", ".join(
            "Section { title: " + rust_str(group.title)
            + ", description: " + ("None" if group.description is None else f"Some({rust_str(group.description)})")
            + ", actions: &[" + ", ".join(str(position[id(a)]) for a in group._group_actions) + "] }"
            for group in current._action_groups
        )
        defaults = ", ".join(f"({rust_str(k)}, {lit(v)})" for k, v in current._defaults.items())
        description = "None" if current.description is None else f"Some({rust_str(current.description)})"
        blocks.append(
            "    Parser {\n"
            f"        prog: {rust_str(current.prog)},\n"
            f"        description: {description},\n"
            f"        actions: &[\n        {actions},\n        ],\n"
            f"        options: &[{options}],\n"
            f"        mutex_groups: &[{groups}],\n"
            f"        sections: &[{sections}],\n"
            f"        defaults: &[{defaults}],\n"
            "    }"
        )
    header = (
        "// @generated by crates/cli/scripts/gen_parser_spec.py from kassiber/cli/main.py.\n"
        "// Do not edit by hand; rerun the script after changing the Python grammar.\n\n"
        "use crate::spec::*;\n\n"
        f"/// Every parser `build_parser()` builds, depth first; index 0 is `kassiber`.\n"
        f"pub(crate) static PARSERS: [Parser; {len(parsers)}] = [\n"
    )
    return header + ",\n".join(blocks) + ",\n];\n"


def main() -> int:
    options = argparse.ArgumentParser()
    options.add_argument("--check", action="store_true")
    args = options.parse_args()
    if sys.version_info[:2] != (3, 11):
        raise SystemExit("run with the locked Python 3.11 interpreter")
    from kassiber.cli.main import build_parser

    parser = build_parser()
    add_rust_only(parser)
    text = render(parser)
    if args.check:
        if TARGET.read_text(encoding="utf-8") != text:
            print(f"{TARGET.relative_to(ROOT)} is stale; rerun {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    TARGET.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

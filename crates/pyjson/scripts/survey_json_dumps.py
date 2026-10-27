#!/usr/bin/env python3
"""Survey how kassiber/ calls json.dumps and json.dump.

kassiber-pyjson reproduces the call variants this finds (crates/pyjson/
README.md). Each call is classified by its keyword arguments, with literal
values as written. A call is counted as hashed when the function around it
also calls hashlib or uuid.uuid5: a heuristic for output that becomes a
stored commitment, so check new hashed sites by hand.

    python3 crates/pyjson/scripts/survey_json_dumps.py [--sites] [root]

It parses the sources and imports nothing from them.
"""

from __future__ import annotations

import argparse
import ast
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]


def _json_names(tree: ast.Module) -> tuple[set[str], dict[str, str]]:
    modules = {"json"}
    functions: dict[str, str] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == "json":
                    modules.add(alias.asname or "json")
        elif isinstance(node, ast.ImportFrom) and node.module == "json":
            for alias in node.names:
                if alias.name in ("dump", "dumps"):
                    functions[alias.asname or alias.name] = alias.name
    return modules, functions


def _called(node: ast.Call, modules: set[str], functions: dict[str, str]) -> str | None:
    func = node.func
    if (
        isinstance(func, ast.Attribute)
        and isinstance(func.value, ast.Name)
        and func.value.id in modules
        and func.attr in ("dump", "dumps")
    ):
        return func.attr
    if isinstance(func, ast.Name) and func.id in functions:
        return functions[func.id]
    return None


def _hashes(function: ast.AST | None) -> bool:
    if function is None:
        return False
    for node in ast.walk(function):
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            if node.value.id == "hashlib" or (node.value.id == "uuid" and node.attr == "uuid5"):
                return True
    return False


def survey(root: Path) -> tuple[Counter, Counter, dict[str, list[str]]]:
    variants: Counter = Counter()
    hashed: Counter = Counter()
    sites: dict[str, list[str]] = defaultdict(list)
    for path in sorted(root.rglob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        modules, functions = _json_names(tree)
        enclosing: dict[ast.AST, ast.AST] = {}
        for function in ast.walk(tree):
            if isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef)):
                for child in ast.walk(function):
                    enclosing[child] = function
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            name = _called(node, modules, functions)
            if name is None:
                continue
            kwargs = sorted(
                f"{kw.arg}={ast.unparse(kw.value)}" if kw.arg else f"**{ast.unparse(kw.value)}"
                for kw in node.keywords
            )
            variant = f"{name}({', '.join(kwargs)})"
            variants[variant] += 1
            if _hashes(enclosing.get(node)):
                hashed[variant] += 1
            sites[variant].append(f"{path.relative_to(root.parent)}:{node.lineno}")
    return variants, hashed, sites


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", nargs="?", type=Path, default=REPO / "kassiber")
    parser.add_argument("--sites", action="store_true", help="list every call site")
    args = parser.parse_args()
    variants, hashed, sites = survey(args.root)
    print("| Calls | Hashed | Keyword arguments |")
    print("| ---: | ---: | --- |")
    for variant, count in sorted(variants.items(), key=lambda item: (-item[1], item[0])):
        print(f"| {count} | {hashed[variant]} | `{variant}` |")
    print(f"\n{sum(variants.values())} calls, {sum(hashed.values())} hashed, {len(variants)} variants")
    if args.sites:
        for variant in sorted(sites):
            print(f"\n{variant}")
            for site in sites[variant]:
                print(f"    {site}")


if __name__ == "__main__":
    main()

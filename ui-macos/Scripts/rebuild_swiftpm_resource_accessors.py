#!/usr/bin/env python3
"""Rebuild SwiftPM resource accessors for a signed macOS app layout.

SwiftPM's command-line executable products look for Bundle.module resources
beside Bundle.main.bundleURL. A signed macOS .app must instead keep those
bundles under Contents/Resources. SwiftPM regenerates accessors during build
planning, so this helper patches them after planning, replays the exact target
compiler commands recorded by SwiftPM, and relinks the executable without
running the planner again.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys


OLD_LOOKUP = "Bundle.main.bundleURL.appendingPathComponent"
NEW_LOOKUP = "(Bundle.main.resourceURL ?? Bundle.main.bundleURL).appendingPathComponent"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--build-dir", required=True, type=Path)
    parser.add_argument("--product", required=True)
    return parser.parse_args()


def load_manifest_commands(manifest: Path) -> dict[str, list[str]]:
    commands: dict[str, list[str]] = {}
    current: str | None = None
    key_pattern = re.compile(r'^  ("(?:[^"\\]|\\.)*"):$')
    args_pattern = re.compile(r"^    args: (\[.*\])$")
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if match := key_pattern.match(line):
            current = json.loads(match.group(1))
            continue
        if current is not None and (match := args_pattern.match(line)):
            commands[current] = json.loads(match.group(1))
    return commands


def resource_modules(build_dir: Path) -> set[str]:
    modules: set[str] = set()
    accessors = sorted(build_dir.glob("*.build/DerivedSources/resource_bundle_accessor.swift"))
    if not accessors:
        raise RuntimeError(f"no generated resource accessors found below {build_dir}")
    for accessor in accessors:
        source = accessor.read_text(encoding="utf-8")
        patched = source.replace(OLD_LOOKUP, NEW_LOOKUP)
        if NEW_LOOKUP not in patched:
            raise RuntimeError(f"generated accessor has an unsupported shape: {accessor}")
        accessor.write_text(patched, encoding="utf-8")
        modules.add(accessor.parent.parent.name.removesuffix(".build"))
    return modules


def dependency_order(modules: set[str], dependency_map: dict[str, list[str]]) -> list[str]:
    ordered: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(module: str) -> None:
        if module in visited:
            return
        if module in visiting:
            raise RuntimeError(f"resource target dependency cycle at {module}")
        visiting.add(module)
        for dependency in dependency_map.get(module, []):
            if dependency in modules:
                visit(dependency)
        visiting.remove(module)
        visited.add(module)
        ordered.append(module)

    for module in sorted(modules):
        visit(module)
    return ordered


def run(command: list[str], cwd: Path) -> None:
    # SwiftPM asks swiftc for machine-readable progress. Direct replay does not
    # have SwiftPM's decoder, so use normal diagnostics and keep build logs
    # human-sized.
    command = [argument for argument in command if argument != "-parseable-output"]
    print("\n> " + " ".join(shlex.quote(argument) for argument in command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def main() -> int:
    arguments = parse_args()
    build_dir = arguments.build_dir.resolve()
    native_root = build_dir.parent.parent.parent
    manifest = build_dir.parent.parent / "release.yaml"
    description_path = build_dir / "description.json"
    if not manifest.is_file() or not description_path.is_file():
        raise RuntimeError("run the release SwiftPM build before rebuilding resource accessors")

    description = json.loads(description_path.read_text(encoding="utf-8"))
    commands = load_manifest_commands(manifest)
    modules = resource_modules(build_dir)
    ordered_modules = dependency_order(modules, description["targetDependencyMap"])

    for module in ordered_modules:
        prefix = f"C.{module}-"
        candidates = [
            command
            for key, command in commands.items()
            if key.startswith(prefix) and key.endswith("-release.module")
        ]
        if len(candidates) != 1:
            raise RuntimeError(f"expected one release compiler command for {module}, found {len(candidates)}")
        run(candidates[0], native_root)

    product_path = str(build_dir / arguments.product)
    link_commands = []
    for command in commands.values():
        if "-emit-executable" not in command or "-o" not in command:
            continue
        output_index = command.index("-o") + 1
        if output_index < len(command) and command[output_index] == product_path:
            link_commands.append(command)
    if len(link_commands) != 1:
        raise RuntimeError(
            f"expected one executable linker command for {product_path}, found {len(link_commands)}"
        )
    run(link_commands[0], native_root)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"resource accessor rebuild failed: {error}", file=sys.stderr)
        raise SystemExit(1)

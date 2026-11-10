"""Prebuilt Rust test binaries, refused when stale.

The parity tests run the `kassiber` binary, examples such as
`publication_fixture` from its directory, and probes named by environment
variables. Rebuilding one leaves the others behind, and an older binary's
output then differs from Python's looking like a port gap (one missing
`wallet_utxos` write showed up as a chain-index clock of 9 instead of 10).

Cargo writes the workspace sources each binary was built from into the dep-info
file beside it (`<binary>.d`). A binary older than any of those sources or than
`Cargo.lock`, missing one of them, or without its dep-info is refused with the
build to run. Sources it does not depend on do not matter, since cargo leaves
an up-to-date binary untouched.

Fresh sources do not prove the right build: another cargo invocation (a
workspace `cargo test` unifies test features across every crate) relinks the
same binary or example with a different feature set, and dep-info stays fresh.
A build step may record the features it built beside the binary
(`<binary>.build.json`, written by `record`). The binary is then refused once
it is no longer the file that build produced.
"""
import argparse
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Environment variables naming a prebuilt binary or example (tests/conftest.py
# checks them once per session). Variables a test sets for its own child
# processes (KASSIBER_HYBRID_RUST_BIN, KASSIBER_TEST_JOURNAL_PREBUILD_BINARY,
# which may name a fake) are not session inputs.
BINARY_ENV = (
    "KASSIBER_RUST_BIN",
    "KASSIBER_TEST_CUSTODY_LOADER_BIN",
    "KASSIBER_TEST_JOURNAL_PROCESS_BIN",
    "KASSIBER_ANALYSIS_INDEX_BIN",
    "KASSIBER_ANALYSIS_DURABLE_BIN",
    "KASSIBER_ANALYSIS_OPS_BIN",
    "KASSIBER_ANALYSIS_TRAVERSAL_BIN",
    "KASSIBER_ANALYSIS_ENTROPY_BIN",
    "KASSIBER_ANALYSIS_DATASETS_BIN",
    "KASSIBER_ANALYSIS_ANALYTICS_BIN",
    "KASSIBER_CHAIN_ANALYSIS_READS_BIN",
    "KASSIBER_FEATURES_BIN",
    "KASSIBER_PSBT_BIN",
    "KASSIBER_PSBT_PRIVACY_BIN",
    "KASSIBER_PSBT_TRANSPORT_BIN",
    "KASSIBER_LEGACY_PRIVACY_BIN",
    "KASSIBER_LIGHTNING_FIXTURE_BIN",
    "KASSIBER_XLSX_SERVICE_BIN",
    "KASSIBER_XLSX_RENDER_BIN",
    "KASSIBER_SOF_MUTATION_SERVICE_BIN",
    "KASSIBER_SOF_CASE_SERVICE_BIN",
    "KASSIBER_RUST_FRESHNESS_FIXTURE",
    "KASSIBER_RUST_FRESHNESS_SYNC_FIXTURE",
    "KASSIBER_RUST_GRAPH_FIXTURE",
    "KASSIBER_DOCUMENT_FIXTURE",
    "KASSIBER_RUST_AI_BROKER_EXAMPLE",
)


def _dependencies(dep_info):
    """The paths after the target in a Makefile-style cargo dep-info file."""
    line = dep_info.read_text().splitlines()[0]
    _, _, deps = line.partition(": ")
    paths, current, escaped = [], "", False
    for char in deps:
        if escaped:
            current += char
            escaped = False
        elif char == "\\":
            escaped = True
        elif char == " ":
            if current:
                paths.append(Path(current))
            current = ""
        else:
            current += char
    if current:
        paths.append(Path(current))
    return paths


def stale(binary, root=None):
    """Why `binary` does not match the current sources, or None."""
    root = ROOT if root is None else root
    binary = Path(binary).resolve()
    dep_info = binary.with_suffix(".d") if binary.suffix == ".exe" else binary.with_name(binary.name + ".d")
    if not dep_info.is_file():
        return f"has no cargo dep-info ({dep_info.name}) to check it against the sources"
    built = binary.stat().st_mtime
    for path in [root / "Cargo.lock", *_dependencies(dep_info)]:
        if not path.exists():
            return f"was built from {path}, which no longer exists"
        if path.stat().st_mtime > built:
            return f"predates {path}"
    recorded = _build_record(binary)
    if recorded is not None and recorded.get("identity") != _identity(binary):
        return (f"was relinked after its build with {recorded.get('features')!r} was recorded "
                "(another cargo invocation, such as a workspace test, built a different feature set)")
    return None


def _build_record_path(binary):
    return binary.with_name(binary.name + ".build.json")


def _identity(binary):
    """The file a build produced: cargo relinks a new inode on every rebuild."""
    status = binary.stat()
    return [status.st_ino, status.st_size, status.st_mtime_ns]


def _build_record(binary):
    path = _build_record_path(binary)
    return json.loads(path.read_text()) if path.is_file() else None


def record(binary, features):
    """Note beside `binary` the features the build that produced it used."""
    binary = Path(binary).resolve()
    _build_record_path(binary).write_text(
        json.dumps({"features": features, "identity": _identity(binary)}) + "\n"
    )


def checked(binary, build):
    """`binary` as a Path, refused when it exists but is stale.

    A missing binary passes through, so the callers' own skips and build
    hints still apply.
    """
    binary = Path(binary)
    if binary.is_file():
        reason = stale(binary)
        assert reason is None, f"{binary} {reason}; rebuild it: {build}"
    return binary


def fixture_binary(name, *, crate="kassiber-wallets", features="test-harness"):
    """`examples/<name>` beside KASSIBER_RUST_BIN, built and not stale."""
    binary = Path(os.environ["KASSIBER_RUST_BIN"]).resolve().parent / "examples" / name
    build = f"cargo build -p {crate} --example {name} --features {features} --offline --locked"
    assert binary.is_file(), f"build it: {build}"
    return checked(binary, build)


def stale_environment_binaries(environ=os.environ):
    """`(variable, path, reason)` for each stale binary the environment names."""
    found = []
    for variable in BINARY_ENV:
        value = environ.get(variable)
        if value and Path(value).is_file():
            reason = stale(Path(value))
            if reason is not None:
                found.append((variable, value, reason))
    return found


def main(argv=None):
    parser = argparse.ArgumentParser(description="Record the features a build used beside its binaries.")
    sub = parser.add_subparsers(dest="command", required=True)
    rec = sub.add_parser("record")
    rec.add_argument("--features", required=True, help="the build, e.g. '-p kassiber --features test-harness'")
    rec.add_argument("binaries", nargs="+", type=Path)
    args = parser.parse_args(argv)
    for binary in args.binaries:
        record(binary, args.features)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

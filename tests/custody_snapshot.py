"""Record and replay the product's raw custody v1 request locally.

KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR explicitly enables ephemeral recording.
KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1 additionally replays the request in a
fresh process under another hash seed, without a database image. The product
loader owns all input facts; this module only serializes stages and results.

    uv run --locked python -m tests.custody_snapshot check DIR
    uv run --locked python -m tests.custody_snapshot compare DIR_A DIR_B
    uv run --locked python -m tests.custody_snapshot refresh DIR NEW_DIR
    uv run --locked python -m tests.custody_snapshot check-rust NEW_DIR --permuted
"""

from __future__ import annotations

import argparse
import dataclasses
from datetime import date, datetime
from decimal import Decimal
import enum
import hashlib
import itertools
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading
from typing import Any, Iterable, Mapping

from kassiber.core import custody_gaps, custody_journal, ownership, source_overlap
from kassiber.core.onchain import parse_ownership_tx


ENV_VAR = "KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR"
# Also rebuild every recorded build in a fresh process under another hash
# seed and fail on any difference.
VERIFY_ENV_VAR = "KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY"
SCHEMA = "kassiber-custody-snapshot"
VERSION = 1

# Values under these configuration keys must never appear in a snapshot.
_SECRET_KEY_MARKERS = (
    "descriptor", "xpub", "tpub", "ypub", "zpub", "upub", "vpub", "secret",
    "seed", "blinding", "private", "token", "password", "passphrase",
    "macaroon", "rune", "mnemonic", "api_key", "auth",
)
_MIN_SECRET_LENGTH = 16
_WALLET_CONFIG_KEYS = frozenset({"config_json", "wallet_config_json"})

_SEQUENCE = itertools.count()
_SEQUENCE_LOCK = threading.Lock()


# -- JSON encoding ----------------------------------------------------------


def to_jsonable(value: Any, *, redact_config: bool = True) -> Any:
    """Encode build values as plain JSON, tagging what JSON cannot carry.

    Sets are sorted so the document does not depend on hash order; mappings
    keep their insertion order, and mappings with non-string keys become
    ``{"$pairs": [[key, value], ...]}``. Row dicts carry the wallet's whole
    configuration text, descriptors included, into later stages (even the
    finalized tax rows); recordings reduce such values to facts by default.
    Stage comparisons explicitly disable that redaction to compare exact text.
    """
    def encode(item):
        return to_jsonable(item, redact_config=redact_config)

    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, enum.Enum):
        return {"$enum": f"{type(value).__name__}.{value.name}", "value": encode(value.value)}
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return value if value == value and value not in (float("inf"), float("-inf")) else {"$float": repr(value)}
    if isinstance(value, Decimal):
        return {"$decimal": str(value)}
    if isinstance(value, (datetime, date)):
        return {"$datetime": value.isoformat()}
    if isinstance(value, (bytes, bytearray)):
        return {"$bytes": bytes(value).hex()}
    if isinstance(value, sqlite3.Row) or (
        not isinstance(value, Mapping) and callable(getattr(value, "keys", None))
        and callable(getattr(value, "__getitem__", None))
    ):
        # sqlite3.Row and the SQLCipher driver's row type.
        return encode({key: value[key] for key in value.keys()})
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {
            "$type": type(value).__name__,
            **{item.name: encode(getattr(value, item.name)) for item in dataclasses.fields(value)},
        }
    if isinstance(value, Mapping):
        if all(isinstance(key, str) for key in value):
            return {
                key: (
                    {"$wallet_config": config_facts(item)}
                    if redact_config and key in _WALLET_CONFIG_KEYS and (item is None or isinstance(item, str))
                    else encode(item)
                )
                for key, item in value.items()
            }
        return {"$pairs": [[encode(key), encode(item)] for key, item in value.items()]}
    if isinstance(value, (set, frozenset)):
        items = [encode(item) for item in value]
        return {"$set": sorted(items, key=_canonical_text)}
    if isinstance(value, (list, tuple)):
        return [encode(item) for item in value]
    raise TypeError(f"custody snapshot cannot encode {type(value).__name__}")


def _canonical_text(value: Any) -> str:
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":"), allow_nan=False)


def digest(value: Any) -> str:
    return hashlib.sha256(_canonical_text(value).encode("ascii")).hexdigest()


# -- redaction --------------------------------------------------------------


from kassiber.core.custody_snapshot import config_facts, encode_owned_index


def _secret_strings(value: Any, *, under_secret_key: bool = False) -> Iterable[str]:
    if isinstance(value, Mapping):
        for key, item in value.items():
            marked = under_secret_key or any(marker in str(key).lower() for marker in _SECRET_KEY_MARKERS)
            yield from _secret_strings(item, under_secret_key=marked)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from _secret_strings(item, under_secret_key=under_secret_key)
    elif isinstance(value, str) and under_secret_key and len(value) >= _MIN_SECRET_LENGTH:
        yield value


def wallet_material(conn: sqlite3.Connection, profile_id: str) -> set[str]:
    """Descriptor, key and policy strings of a book, to prove none leaked."""

    found: set[str] = set()
    for row in conn.execute("SELECT config_json FROM wallets WHERE profile_id = ?", (profile_id,)):
        try:
            found.update(_secret_strings(json.loads(row[0] or "{}")))
        except (TypeError, ValueError):
            continue
    try:
        epochs = conn.execute(
            "SELECT e.private_material_json FROM wallet_policy_epochs e "
            "JOIN wallets w ON w.id = e.wallet_id WHERE w.profile_id = ?",
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        epochs = []
    for row in epochs:
        try:
            found.update(_secret_strings(json.loads(row[0] or "{}")))
        except (TypeError, ValueError):
            continue
    return found


# -- owned index ------------------------------------------------------------


def _match(match: Any) -> dict[str, Any]:
    # Derivation paths and address positions are not read by the custody
    # computation and stay out.
    return {
        "wallet_id": match.wallet_id,
        "wallet_label": match.wallet_label,
        "account": match.account,
        "chain": match.chain,
        "network": match.network,
        "source": match.source,
        "wallet_kind": match.wallet_kind,
    }


class _RecordingOwnedIndex(ownership.OwnedIndex):
    """The same index, noting every lookup the computation makes."""

    def __init__(self, index: ownership.OwnedIndex, log: list[Any], full=None) -> None:
        super().__init__(**{item.name: getattr(index, item.name) for item in dataclasses.fields(index)})
        self._log = log
        self._full = full or index

    def _note(self, method: str, args: list[Any], result: Any) -> Any:
        if method in {"lookup_outpoint", "lookup_txid_wallets"}:
            full_result = getattr(self._full, method)(args[0], chain=args[1], network=args[2])
        else:
            full_result = getattr(self._full, method)(*args)
        observed = full_result
        if isinstance(observed, (set, frozenset)):
            encoded = sorted([list(item) for item in observed])
        else:
            encoded = [_match(item) for item in observed]
        self._log.append([method, args, encoded])
        return result

    def lookup_address(self, address):
        return self._note("lookup_address", [address], super().lookup_address(address))

    def lookup_script(self, script_hex):
        return self._note("lookup_script", [script_hex], super().lookup_script(script_hex))

    def lookup_outpoint(self, outpoint, *, chain=None, network=None):
        return self._note(
            "lookup_outpoint",
            [outpoint, chain, network],
            super().lookup_outpoint(outpoint, chain=chain, network=network),
        )

    def lookup_txid_wallets(self, txid, *, chain=None, network=None):
        return self._note(
            "lookup_txid_wallets",
            [txid, chain, network],
            super().lookup_txid_wallets(txid, chain=chain, network=network),
        )


def restricted_owned_index(index, rows):
    return encode_owned_index(index, rows)


def uncovered_lookups(snapshot: Mapping[str, Any]) -> list[Any]:
    """Lookups that found something the restricted index does not hold."""

    from kassiber.core.custody_snapshot import decode_owned_index
    index = decode_owned_index((snapshot.get("input") or {}).get("owned_index"))
    missing = []
    for method, args, expected in (snapshot.get("observed") or {}).get("owned_index_lookups", []):
        if index is None:
            actual = []
        else:
            if method in {"lookup_outpoint", "lookup_txid_wallets"}:
                found = getattr(index, method)(args[0], chain=args[1], network=args[2])
            else:
                found = getattr(index, method)(*args)
            actual = sorted([list(item) for item in found]) if isinstance(found, (set, frozenset)) else [_match(item) for item in found]
        if actual != expected:
            missing.append([method, args, expected])
    return missing


# -- raw authored components ------------------------------------------------


# -- the recorder -----------------------------------------------------------


def owner_only_file(path):
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    return os.fdopen(fd, "w", encoding="ascii")


class SnapshotRecorder:
    """Observes one build and writes its snapshot document."""

    def __init__(self, builder: custody_journal.CustodyJournalBuilder, directory: Path) -> None:
        self.builder = builder
        self.directory = directory
        self.stages: dict[str, Any] = {}
        self.inputs: dict[str, Any] = {}
        self.lookups: list[Any] = []
        self.owned_index_present = False
        self.restricted_index: dict[str, Any] | None = None

    # Observer protocol --------------------------------------------------

    def owned_index(self, index):
        if index is None:
            return None
        self.owned_index_present = True
        self._index = index
        full = index
        if self.builder.conn is not None:
            full, _ = ownership.build_owned_index(self.builder.conn, self.builder.profile_id, ownership.load_profile_wallets(self.builder.conn, self.builder.profile_id))
        return _RecordingOwnedIndex(index, self.lookups, full)

    def stage(self, name: str, value: Any) -> None:
        if name == "snapshot":
            # The product loader is the single source of raw v1 facts.
            self.inputs = value
        else:
            self.stages[name] = to_jsonable(value)

    def finish(self, projection: custody_journal.CustodyJournalProjection) -> None:
        output = {
            item.name: to_jsonable(getattr(projection, item.name))
            for item in dataclasses.fields(projection)
            # Inputs and stage results already in the document.
            if item.name not in {"rows", "rates", "wallet_refs_by_id", "owned_index", "loan_legs", "active_components", "channel_roles"}
        }
        self._write(output=output)

    def fail(self, exc: BaseException) -> None:
        self._write(error={"type": type(exc).__name__, "message": str(exc)})

    # Document ------------------------------------------------------------

    def _document(self, **result: Any) -> dict[str, Any]:
        inputs = self.inputs
        return {
            "schema": SCHEMA, "version": VERSION,
            "reference_commit": "1bfb5738",
            "engine_version": "kassiber-custody/0.1.0; Python reference/1bfb5738",
            "generator": {"kind": "local test fixture", "test": os.environ.get("PYTEST_CURRENT_TEST") or os.environ.get("KASSIBER_TEST_CUSTODY_MODULE")},
            "comparison_policy": "custody-v1 exact stages; normalized wallet configuration; no id/hash/array masking",
            "input_digest": digest({key: value for key, value in inputs.items() if key != "as_of"}),
            "source": {"test": os.environ.get("PYTEST_CURRENT_TEST")},
            "input": inputs,
            "observed": {"owned_index_lookups": self.lookups},
            "stages": self.stages, **result,
        }

    def _write(self, **result: Any) -> None:
        document = self._document(**result)
        text = _canonical_text(document) + "\n"
        leaked = [
            value
            for value in (wallet_material(self.builder.conn, self.builder.profile_id) if self.builder.conn is not None else ())
            if value in text or json.dumps(value, ensure_ascii=True)[1:-1] in text
        ]
        if leaked:
            raise AssertionError("custody snapshot would contain wallet key material")
        replay = _replay_elsewhere(self.builder, document) if _verify_enabled() else None
        with _SEQUENCE_LOCK:
            sequence = next(_SEQUENCE)
        name = f"{document['input_digest'][:24]}-{os.getpid()}-{sequence:06d}"
        with owner_only_file(self.directory / f"{name}.json") as handle:
            handle.write(text)
        if replay is not None and (
            replay["input_digest"] != document["input_digest"]
            or _canonical_text(_comparable(replay)) != _canonical_text(_comparable(document))
        ):
            with owner_only_file(self.directory / f"{name}.replay") as handle:
                handle.write(_canonical_text(replay) + "\n")
            raise AssertionError(
                f"custody build {name} differs when rebuilt in a fresh process under another hash seed"
            )


def _verify_enabled() -> bool:
    return os.environ.get(VERIFY_ENV_VAR, "").strip() == "1"


def _replay_elsewhere(
    builder: custody_journal.CustodyJournalBuilder,
    document: Mapping[str, Any],
) -> dict[str, Any] | None:
    """Replay only the raw request in a child with another hash seed."""

    request = {"snapshot": document["input"], "fusion": "fusion" in document.get("stages", {}), "finalized": "error" in document or "finalized_tax_projection" in document.get("output", {})}
    env = {
        key: value
        for key, value in os.environ.items()
        if key not in {ENV_VAR, VERIFY_ENV_VAR}
    }
    seed = os.environ.get("PYTHONHASHSEED", "")
    env["PYTHONHASHSEED"] = str(int(seed) + 1) if seed.isdigit() else "12345"
    completed = subprocess.run(
        [sys.executable, "-m", "tests.custody_snapshot", "replay-stdin"],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        cwd=Path(__file__).resolve().parent.parent,
        env=env,
        check=False,
    )
    if completed.returncode != 0:
        raise AssertionError(f"custody snapshot replay failed: {completed.stderr[-2000:]}")
    return json.loads(completed.stdout)


def _replay_stdin() -> int:
    """Child side of the verification: build once, print the snapshot."""

    request = json.loads(sys.stdin.read())
    from kassiber.core.custody_compute import compute_custody_decisions, compute_custody_projection
    with tempfile.TemporaryDirectory() as tmp:
        class Builder:
            conn = None
            profile_id = request["snapshot"]["profile"]["id"]
        observer = SnapshotRecorder(Builder(), Path(tmp))
        observer.stage("snapshot", request["snapshot"])
        try:
            compute = compute_custody_projection if request["finalized"] else compute_custody_decisions
            result = compute(request["snapshot"], stage_observer=observer.stage, owned_index_observer=observer.owned_index)
            if request.get("fusion"):
                from tests.custody_fusion_stage import python_fusion
                observer.stage("fusion", python_fusion(request["snapshot"], result.finalized_tax_projection))
        except Exception as exc:
            observer.fail(exc)
        else:
            observer.finish(result)
        written = list(Path(tmp).glob("*.json"))
        if len(written) != 1:
            raise RuntimeError("expected one replay snapshot")
        sys.stdout.write(written[0].read_text(encoding="ascii"))
    return 0


def snapshot_directory() -> Path | None:
    value = os.environ.get(ENV_VAR, "").strip()
    if not value:
        return None
    directory = Path(value)
    if not directory.is_absolute() or not directory.is_dir():
        raise RuntimeError(f"{ENV_VAR} must name an existing absolute directory, not {value!r}")
    return directory


def install(directory: Path):
    """Record every custody build into ``directory``; returns the previous factory."""

    return custody_journal.install_build_observer_factory(
        lambda builder: SnapshotRecorder(builder, directory)
    )


def install_from_environment() -> bool:
    directory = snapshot_directory()
    if directory is None:
        return False
    install(directory)
    return True


# -- checking recorded snapshots --------------------------------------------


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="ascii"))


def check(directory: Path) -> list[str]:
    """Each snapshot parses, re-encodes byte for byte and is self-consistent."""

    problems = []
    for path in sorted(directory.glob("*.json")):
        text = path.read_text(encoding="ascii")
        document = json.loads(text)
        if _canonical_text(document) + "\n" != text:
            problems.append(f"{path.name}: does not round-trip")
        if document.get("schema") != SCHEMA or document.get("version") != VERSION:
            problems.append(f"{path.name}: unknown schema")
        digest_input = {key: value for key, value in document["input"].items() if key != "as_of"}
        if digest(digest_input) != document["input_digest"]:
            problems.append(f"{path.name}: input digest mismatch")
        if ("output" in document) == ("error" in document):
            problems.append(f"{path.name}: needs exactly one of output or error")
        missing = uncovered_lookups(document)
        if missing:
            problems.append(f"{path.name}: {len(missing)} owned-index lookups outside the restricted index")
    return problems


def pairing_key(document: Mapping[str, Any]) -> str:
    """Builds that must agree: same inputs, and the same clock if it matters.

    The clock reaches the computation only through pair-dismissal expiry, so
    builds whose dismissals never expire pair across runs at any ``as_of``.
    """

    dismissals = document["input"].get("pair_dismissals") or []
    if any(item.get("expires_at") for item in dismissals):
        return f"{document['input_digest']}@{document['input'].get('as_of')}"
    return document["input_digest"]


def _comparable(document: Mapping[str, Any]) -> Any:
    result = dict(document.get("output") or {})
    result.pop("as_of", None)
    return {"output": result, "error": document.get("error"), "stages": document.get("stages")}


def compare(first: Path, second: Path) -> tuple[int, int, list[str]]:
    """Builds with the same inputs in two runs must have the same results."""

    def by_digest(directory: Path) -> dict[str, list[dict[str, Any]]]:
        grouped: dict[str, list[dict[str, Any]]] = {}
        for path in sorted(directory.glob("*.json")):
            document = load(path)
            grouped.setdefault(pairing_key(document), []).append(document)
        return grouped

    left, right = by_digest(first), by_digest(second)
    problems = []
    paired = 0
    for key in sorted(set(left) & set(right)):
        results = {_canonical_text(_comparable(item)) for item in (*left[key], *right[key])}
        paired += 1
        if len(results) != 1:
            problems.append(f"{key}: {len(results)} different results for the same inputs")
    for grouped in (left, right):
        for key, items in grouped.items():
            if len({_canonical_text(_comparable(item)) for item in items}) != 1:
                problems.append(f"{key}: repeated builds in one run differ")
    return paired, len(set(left) | set(right)), problems


def check_rust(directory: Path, *, permuted: bool = False):
    """Replay every recorded raw request without SQL and compare its Rust stage."""
    from kassiber.core.custody_compute import compute_evidence_stage, compute_components_stage
    from tests.custody_engine_compare import assert_same, compare_rust, permuted_snapshot, rust_results
    compared = skipped = permutations = 0
    problems = []
    for path in sorted(directory.rglob("*.json")):
        document = load(path)
        if not isinstance(document, Mapping) or document.get("schema") != SCHEMA:
            continue
        request = document.get("input") or {}
        if request.get("schema_version") != VERSION:
            skipped += 1  # A gate refused the build before a request existed.
            continue
        try:
            from kassiber.core.custody_snapshot import require_snapshot_accounting
            from kassiber.tax_policy import require_tax_processing_supported
            try:
                require_snapshot_accounting(request)
                require_tax_processing_supported(request["profile"])
                from kassiber.core.custody_journal import rates_from_transaction_rows
                rates_from_transaction_rows(request["rate_rows"])
                components, blockers = compute_components_stage(request)
            except Exception as exc:
                rust_results(request, python_error=exc)
                compared += 1
                if permuted:
                    rust_results(permuted_snapshot(request), python_error=exc)
                    permutations += 1
                continue
            component_stage = {"components": components, "blockers": blockers}
            recorded_components = document.get("stages", {}).get("components")
            if recorded_components is not None:
                assert_same(recorded_components, component_stage, "recorded/replayed Python components")
            evidence = compute_evidence_stage(request)
            recorded = document.get("stages", {}).get("evidence")
            if recorded is not None:
                assert_same(recorded, evidence, "recorded/replayed Python evidence")
            from tests.custody_finalized_stage import python_stages
            try:
                stages = python_stages(request)
                from tests.custody_fusion_stage import add_fusion
                add_fusion(request, stages)
            except Exception as exc:
                rust_results(request, python_error=exc)
                compared += 1
                if permuted:
                    rust_results(permuted_snapshot(request), python_error=exc)
                    permutations += 1
                continue
            for name in ("interpretation", "gaps", "quantity", "finalized_projection", "fusion"):
                recorded = document.get("stages", {}).get(name)
                if recorded is not None:
                    assert_same(recorded, to_jsonable(stages[name]) if name == "finalized_projection" else stages[name], "recorded/replayed Python " + name)
            implemented, _ = compare_rust(request, evidence, python_stages=stages)
            if not {"components", "evidence", "interpretation", "gaps", "quantity", "finalized_projection", "fusion"}.issubset(implemented):
                raise AssertionError("Rust complete custody stage inventory is unavailable")
            compared += 1
            if permuted:
                altered = permuted_snapshot(request)
                compare_rust(altered, compute_evidence_stage(altered))
                permutations += 1
        except Exception as exc:
            problems.append(f"{path.relative_to(directory)}: {type(exc).__name__}: {exc}")
    return compared, permutations, skipped, problems


def refresh(source: Path, target: Path) -> tuple[int, int]:
    """Copy every recording with its stages recomputed by the current Python.

    Recorded stages pin the reference that wrote them; a later reference
    change (a new row field, a retained quarantine) makes `check-rust` stop
    at "recorded/replayed Python" before Rust is compared. The inputs, the
    observed lookups and the recorded output stay as they were. A request a
    gate refused is copied unchanged; one the current Python refuses keeps
    the stages completed before the refusal.
    """
    from kassiber.core.custody_compute import compute_custody_projection
    refreshed = copied = 0
    for path in sorted(source.rglob("*.json")):
        document = load(path)
        destination = target / path.relative_to(source)
        destination.parent.mkdir(parents=True, exist_ok=True)
        request = (document.get("input") or {}) if isinstance(document, Mapping) else {}
        stages: dict[str, Any] = {}
        if isinstance(document, Mapping) and document.get("schema") == SCHEMA and request.get("schema_version") == VERSION:
            try:
                compute_custody_projection(request, stage_observer=lambda name, value: name == "snapshot" or stages.setdefault(name, to_jsonable(value)))
            except Exception:
                pass  # A refusal keeps the stages completed before it.
        if stages:
            # The recorded stage set, each as the current reference computes it.
            document = dict(document, stages={name: stages.get(name, value) for name, value in document.get("stages", {}).items()})
            refreshed += 1
        else:
            copied += 1
        destination.write_text(_canonical_text(document) + "\n", encoding="ascii")
    return refreshed, copied


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    check_parser = sub.add_parser("check")
    check_parser.add_argument("directory", type=Path)
    compare_parser = sub.add_parser("compare")
    compare_parser.add_argument("first", type=Path)
    compare_parser.add_argument("second", type=Path)
    rust_parser = sub.add_parser("check-rust")
    rust_parser.add_argument("directory", type=Path)
    rust_parser.add_argument("--permuted", action="store_true")
    refresh_parser = sub.add_parser("refresh")
    refresh_parser.add_argument("source", type=Path)
    refresh_parser.add_argument("target", type=Path)
    sub.add_parser("replay-stdin")
    args = parser.parse_args(argv)
    if args.command == "replay-stdin":
        return _replay_stdin()
    if args.command == "refresh":
        if args.target.exists():
            raise SystemExit(f"{args.target} exists; refresh writes a new directory")
        refreshed, copied = refresh(args.source, args.target)
        print(f"{refreshed} recordings refreshed, {copied} copied unchanged")
        return 0
    if args.command == "check-rust":
        compared, permutations, skipped, problems = check_rust(args.directory, permuted=args.permuted)
        print(f"{compared} recorded requests compared, {permutations} permutations, {skipped} pre-request refusals, {len(problems)} differences")
        for problem in problems[:50]:
            print(problem)
        return int(bool(problems) or compared == 0)
    if args.command == "check":
        count = len(list(args.directory.glob("*.json")))
        problems = check(args.directory)
        print(f"{count} snapshots, {len(problems)} problems")
    else:
        paired, total, problems = compare(args.first, args.second)
        print(f"{paired} input digests in both runs of {total}, {len(problems)} problems")
        if not paired:
            problems.append("no build had the same inputs in both runs; nothing was compared")
    for problem in problems[:50]:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

"""Record what each custody build consumed and produced (draft snapshot v0).

Set ``KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR`` to an existing directory and run
pytest: ``tests/conftest.py`` installs this recorder through
``custody_journal.install_build_observer_factory`` and every in-process
``CustodyJournalBuilder.build_custody_projection`` call writes one JSON
document there. Product code reads no environment variable for this and,
without the recorder, observes nothing.

The document carries the build's raw inputs (``input``), what the Python
reference looked up in the owned index (``observed``), its intermediate stage
results (``stages``) and its decisions and finalized projection (``output``),
or the ``error`` it raised. Wallet configuration is reduced to the facts the
computation reads; descriptors, xpubs, keys and retired policy material are
never written, and every document is checked for them before it is saved.
The schema is described in docs/plan/21-rust-backend.md ("R2 custody
snapshot").

With ``KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1`` as well, every recorded
build is rebuilt in a fresh process under another ``PYTHONHASHSEED`` from the
same book state and ``as_of``; any difference in inputs or results fails the
test. The book travels to that process in memory only.

    uv run --locked python -m tests.custody_snapshot check DIR
    uv run --locked python -m tests.custody_snapshot compare DIR_A DIR_B
"""

from __future__ import annotations

import argparse
import base64
import dataclasses
from datetime import date, datetime
import decimal
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
VERSION = 0

# Wallet configuration keys the custody computation reads. Everything else
# (descriptors, xpubs, address lists, backend settings) stays out.
CONFIG_FACT_KEYS = (
    "chain",
    "network",
    "bitcoin_network",
    "chain_network",
    "chain_instance_id",
    "samourai",
)
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


def to_jsonable(value: Any) -> Any:
    """Encode build values as plain JSON, tagging what JSON cannot carry.

    Sets are sorted so the document does not depend on hash order; mappings
    keep their insertion order, and mappings with non-string keys become
    ``{"$pairs": [[key, value], ...]}``. Row dicts carry the wallet's whole
    configuration text, descriptors included, into later stages (even the
    finalized tax rows); every such value is reduced to its facts.
    """

    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, enum.Enum):
        return {"$enum": f"{type(value).__name__}.{value.name}", "value": to_jsonable(value.value)}
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
    if isinstance(value, sqlite3.Row):
        return {key: to_jsonable(value[key]) for key in value.keys()}
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {
            "$type": type(value).__name__,
            **{item.name: to_jsonable(getattr(value, item.name)) for item in dataclasses.fields(value)},
        }
    if isinstance(value, Mapping):
        if all(isinstance(key, str) for key in value):
            return {
                key: (
                    {"$wallet_config": config_facts(item)}
                    if key in _WALLET_CONFIG_KEYS and (item is None or isinstance(item, str))
                    else to_jsonable(item)
                )
                for key, item in value.items()
            }
        return {"$pairs": [[to_jsonable(key), to_jsonable(item)] for key, item in value.items()]}
    if isinstance(value, (set, frozenset)):
        items = [to_jsonable(item) for item in value]
        return {"$set": sorted(items, key=_canonical_text)}
    if isinstance(value, (list, tuple)):
        return [to_jsonable(item) for item in value]
    raise TypeError(f"custody snapshot cannot encode {type(value).__name__}")


def _canonical_text(value: Any) -> str:
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":"), allow_nan=False)


def digest(value: Any) -> str:
    return hashlib.sha256(_canonical_text(value).encode("ascii")).hexdigest()


# -- redaction --------------------------------------------------------------


def config_facts(config_json: Any) -> Any:
    """The wallet configuration facts the custody computation reads."""

    if config_json in (None, ""):
        return None
    try:
        value = json.loads(config_json)
    except (TypeError, ValueError):
        return {"$unparsable": True}
    if not isinstance(value, dict):
        return {"$not_an_object": type(value).__name__}
    facts = {key: value[key] for key in CONFIG_FACT_KEYS if key in value}
    # custody_interpreters gates its Samourai pass on this substring of the
    # raw text; keep the bit, not the text.
    facts["$text_mentions_samourai"] = "samourai" in str(config_json).lower()
    return facts


def _row_without_config(row: Mapping[str, Any]) -> dict[str, Any]:
    return {
        key: value
        for key, value in dict(row).items()
        if key not in {"config_json", "wallet_config_json", "_closed_confirmation_observation"}
    }


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

    def __init__(self, index: ownership.OwnedIndex, log: list[Any]) -> None:
        super().__init__(**{item.name: getattr(index, item.name) for item in dataclasses.fields(index)})
        self._log = log

    def _note(self, method: str, args: list[Any], result: Any) -> Any:
        if isinstance(result, (set, frozenset)):
            encoded = sorted([list(item) for item in result])
        else:
            encoded = [_match(item) for item in result]
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


def graph_keys(rows: Iterable[Mapping[str, Any]]) -> dict[str, set[str]]:
    """Scripts, outpoints and previous txids the rows' stored graphs name."""

    scripts: set[str] = set()
    outpoints: set[str] = set()
    txids: set[str] = set()
    for row in rows:
        parsed = parse_ownership_tx(row.get("raw_json"))
        if not parsed:
            continue
        for entry in parsed.get("inputs") or ():
            if entry.get("script"):
                scripts.add(str(entry["script"]).lower())
            outpoint = str(entry.get("outpoint") or "").lower()
            if outpoint:
                outpoints.add(outpoint)
                txids.add(outpoint.split(":", 1)[0])
        for entry in parsed.get("outputs") or ():
            if entry.get("script"):
                scripts.add(str(entry["script"]).lower())
    return {"scripts": scripts, "outpoints": outpoints, "txids": txids}


def restricted_owned_index(index: ownership.OwnedIndex, rows: Iterable[Mapping[str, Any]]) -> dict[str, Any]:
    keys = graph_keys(rows)
    scoped_txids: dict[str, list[list[Any]]] = {}
    for (chain, network, txid), wallets in index.txid_wallets_by_scope.items():
        if txid in keys["txids"]:
            scoped_txids.setdefault(txid, []).append([chain, network, sorted([list(item) for item in wallets])])
    return {
        "by_script": {
            script: [_match(item) for item in index.by_script[script]]
            for script in sorted(keys["scripts"])
            if script in index.by_script
        },
        "by_outpoint": {
            outpoint: [_match(item) for item in index.by_outpoint[outpoint]]
            for outpoint in sorted(keys["outpoints"])
            if outpoint in index.by_outpoint
        },
        "txid_wallets": {
            txid: {
                "unscoped": sorted([list(item) for item in index.txid_wallets.get(txid, set())]),
                "scoped": sorted(scoped_txids.get(txid, [])),
            }
            for txid in sorted(keys["txids"])
            if txid in index.txid_wallets or txid in scoped_txids
        },
    }


def uncovered_lookups(snapshot: Mapping[str, Any]) -> list[Any]:
    """Lookups that found something the restricted index does not hold."""

    index = (snapshot.get("input") or {}).get("owned_index")
    lookups = (snapshot.get("observed") or {}).get("owned_index_lookups") or []
    if index is None:
        return [lookup for lookup in lookups if lookup[2]]
    missing = []
    for method, args, result in lookups:
        if not result:
            continue
        key = str(args[0] or "").lower()
        covered = {
            "lookup_script": key in index["by_script"],
            "lookup_outpoint": key in index["by_outpoint"],
            "lookup_txid_wallets": key in index["txid_wallets"],
        }.get(method, False)
        if not covered:
            missing.append([method, args, result])
    return missing


# -- raw authored components ------------------------------------------------


def _rows(conn: sqlite3.Connection, sql: str, params: Iterable[Any]) -> list[dict[str, Any]]:
    try:
        return [dict(row) for row in conn.execute(sql, tuple(params)).fetchall()]
    except sqlite3.OperationalError:
        return []


def authored_components(conn: sqlite3.Connection, profile_id: str) -> dict[str, Any]:
    """Authored-active components as stored, with the anchors they name."""

    headers = _rows(
        conn,
        "SELECT * FROM custody_components WHERE profile_id = ? AND state = 'active' "
        "ORDER BY created_at, revision, id",
        (profile_id,),
    )
    children: dict[str, list[dict[str, Any]]] = {
        "legs": [], "allocations": [], "economic_terms": [], "evidence_commitments": [],
    }
    tables = {
        "legs": "custody_component_legs",
        "allocations": "custody_component_allocations",
        "economic_terms": "custody_component_economic_terms",
        "evidence_commitments": "custody_component_evidence_commitments",
    }
    for header in headers:
        for name, table in tables.items():
            children[name].extend(
                _rows(conn, f"SELECT * FROM {table} WHERE component_id = ? ORDER BY ordinal, id", (header["id"],))
            )
    anchor_ids = sorted(
        {
            str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
            for leg in children["legs"]
            if leg.get("anchor_transaction_id") or leg.get("transaction_id")
        }
    )
    anchors = []
    for start in range(0, len(anchor_ids), 500):
        chunk = anchor_ids[start:start + 500]
        anchors.extend(
            _rows(
                conn,
                "SELECT t.*, w.kind AS wallet_kind FROM transactions t "
                "LEFT JOIN wallets w ON w.id = t.wallet_id "
                f"WHERE t.id IN ({','.join('?' for _ in chunk)}) ORDER BY t.id",
                chunk,
            )
        )
    present = {str(row["id"]) for row in anchors}
    return {
        "headers": headers,
        **children,
        "anchors": anchors,
        "missing_anchor_ids": [item for item in anchor_ids if item not in present],
    }


# -- the recorder -----------------------------------------------------------


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
        return _RecordingOwnedIndex(index, self.lookups)

    def stage(self, name: str, value: Any) -> None:
        conn = self.builder.conn
        profile_id = self.builder.profile_id
        if name == "transactions":
            self._rows = value
            self.inputs["transactions"] = [to_jsonable(_row_without_config(row)) for row in value]
        elif name == "wallet_refs":
            configs = {
                str(row["id"]): row["config_json"]
                for row in conn.execute("SELECT id, config_json FROM wallets WHERE profile_id = ?", (profile_id,))
            }
            self.inputs["wallets"] = [
                {**to_jsonable(ref), "config": to_jsonable(config_facts(configs.get(str(wallet_id))))}
                for wallet_id, ref in value.items()
            ]
        elif name == "lightning_channel_records":
            self.inputs["lightning_channel_records"] = [to_jsonable(_row_without_config(row)) for row in value]
        elif name == "materialized_components":
            self.inputs["authored_components"] = to_jsonable(authored_components(conn, profile_id))
            self.stages[name] = to_jsonable(value)
        elif name in {"loan_legs", "pair_dismissals", "component_evidence_snapshots", "dismissed_gap_fingerprints", "as_of"}:
            self.inputs[name] = to_jsonable(value)
        elif name == "migration_quarantines":
            self.inputs["migration_issues"] = to_jsonable(
                _rows(
                    conn,
                    "SELECT * FROM custody_authored_migration_issues "
                    "WHERE profile_id = ? AND resolved_at IS NULL ORDER BY created_at, id",
                    (profile_id,),
                )
            )
            self.stages[name] = to_jsonable(value)
        elif name == "receipt_reconciliations":
            groups = source_overlap._receipt_overlap_candidate_groups(self._rows)
            self.inputs["receipt_reconciliation_sources"] = (
                [
                    {
                        key: getattr(source, key)
                        for key in (
                            "wallet_id", "wallet_label", "wallet_kind", "chain", "network",
                            "script_pubkey", "source", "deprecated", "active_transaction_count",
                        )
                    }
                    for source in source_overlap.build_profile_source_index(conn, profile_id).sources
                ]
                if groups
                else None
            )
            self.stages[name] = to_jsonable(value)
        elif name == "gap_search":
            result, legs = value
            active = int(conn.execute(
                "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0", (profile_id,),
            ).fetchone()[0])
            large_book = active > custody_gaps.DEFAULT_MAX_INPUT_ROWS
            self.inputs["gap_search"] = {
                "active_transaction_count": active,
                "large_book": large_book,
                # Small books search every active row; large books a SQL-selected
                # worklist of them, in this order.
                "worklist_transaction_ids": [leg.id for leg in legs] if large_book else None,
            }
            self.stages["gap_search_result"] = to_jsonable(result)
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
        builder = self.builder
        context = decimal.getcontext()
        index = getattr(self, "_index", None)
        rows = getattr(self, "_rows", None)
        inputs = {
            "profile": to_jsonable(dict(builder.profile)),
            "decimal_context": {"prec": context.prec, "rounding": context.rounding},
            **self.inputs,
            "owned_index": (
                restricted_owned_index(index, rows or ()) if index is not None else None
            ),
        }
        # The clock is an input, but it differs on every run of a test; keep
        # it out of the digest that pairs builds across runs.
        digest_input = {key: value for key, value in inputs.items() if key != "as_of"}
        return {
            "schema": SCHEMA,
            "version": VERSION,
            "input_digest": digest(digest_input),
            "source": {"test": os.environ.get("PYTEST_CURRENT_TEST")},
            "input": inputs,
            "observed": {"owned_index_lookups": self.lookups},
            "stages": self.stages,
            **result,
        }

    def _write(self, **result: Any) -> None:
        document = self._document(**result)
        text = _canonical_text(document) + "\n"
        leaked = [
            value
            for value in wallet_material(self.builder.conn, self.builder.profile_id)
            if value in text or json.dumps(value, ensure_ascii=True)[1:-1] in text
        ]
        if leaked:
            raise AssertionError("custody snapshot would contain wallet key material")
        replay = _replay_elsewhere(self.builder, document) if _verify_enabled() else None
        with _SEQUENCE_LOCK:
            sequence = next(_SEQUENCE)
        name = f"{document['input_digest'][:24]}-{os.getpid()}-{sequence:06d}"
        with open(self.directory / f"{name}.json", "x", encoding="ascii") as handle:
            handle.write(text)
        if replay is not None and (
            replay["input_digest"] != document["input_digest"]
            or _canonical_text(_comparable(replay)) != _canonical_text(_comparable(document))
        ):
            with open(self.directory / f"{name}.replay", "x", encoding="ascii") as handle:
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
    """Rebuild from the same book state in a child with another hash seed.

    The book is handed over in memory (including the build's uncommitted
    transaction) and never written to disk.
    """

    serialize = getattr(builder.conn, "serialize", None)
    if not isinstance(builder.conn, sqlite3.Connection) or serialize is None:
        return None
    request = {
        "database": base64.b64encode(serialize()).decode("ascii"),
        "profile": document["input"]["profile"],
        "as_of": document["input"].get("as_of"),
    }
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
    image = bytearray(base64.b64decode(request["database"]))
    # A WAL book's header names WAL (format version 2); an in-memory copy has
    # no WAL file, so mark it as a rollback-journal database (version 1).
    if image[18:20] == b"\x02\x02":
        image[18:20] = b"\x01\x01"
    conn = sqlite3.connect(":memory:")
    conn.deserialize(bytes(image))
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    with tempfile.TemporaryDirectory() as tmp:
        install(Path(tmp))
        builder = custody_journal.CustodyJournalBuilder(conn, request["profile"], as_of=request["as_of"])
        failure = None
        try:
            builder.build_custody_projection()
        except Exception as exc:  # the error snapshot carries a build failure
            failure = exc
        written = sorted(Path(tmp).glob("*.json"))
        if len(written) != 1:
            if failure is not None:
                raise failure
            raise SystemExit(f"expected one replay snapshot, found {len(written)}")
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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    check_parser = sub.add_parser("check")
    check_parser.add_argument("directory", type=Path)
    compare_parser = sub.add_parser("compare")
    compare_parser.add_argument("first", type=Path)
    compare_parser.add_argument("second", type=Path)
    sub.add_parser("replay-stdin")
    args = parser.parse_args(argv)
    if args.command == "replay-stdin":
        return _replay_stdin()
    if args.command == "check":
        count = len(list(args.directory.glob("*.json")))
        problems = check(args.directory)
        print(f"{count} snapshots, {len(problems)} problems")
    else:
        paired, total, problems = compare(args.first, args.second)
        print(f"{paired} input digests in both runs of {total}, {len(problems)} problems")
    for problem in problems[:50]:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

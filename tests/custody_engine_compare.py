"""R2 stage comparisons. Test-only; product code never imports this module."""
from __future__ import annotations
from dataclasses import replace
import json
from collections.abc import Mapping

from kassiber.core.custody_snapshot import encode_owned_index
from tests.custody_snapshot import to_jsonable


def comparable(value):
    def clean(item):
        if isinstance(item, Mapping):
            return {key: clean(val) for key, val in item.items() if key != "config_text_mentions_samourai"}
        if isinstance(item, list):
            return [clean(val) for val in item]
        return item
    if hasattr(value, "owned_index"):
        value = replace(value, owned_index=encode_owned_index(value.owned_index, value.rows))
    return clean(to_jsonable(value))


def assert_same(expected, actual, label="custody"):
    left, right = comparable(expected), comparable(actual)
    def difference(a, b, path):
        if type(a) is not type(b):
            return f"{path}: {type(a).__name__} != {type(b).__name__}"
        if isinstance(a, dict):
            if a.keys() != b.keys():
                return f"{path}: keys {sorted(a.keys() ^ b.keys())}"
            for key in a:
                found = difference(a[key], b[key], f"{path}.{key}")
                if found:
                    return found
        elif isinstance(a, list):
            if len(a) != len(b):
                return f"{path}: length {len(a)} != {len(b)}"
            for i, (x, y) in enumerate(zip(a, b)):
                found = difference(x, y, f"{path}[{i}]")
                if found:
                    return found
        elif a != b:
            return f"{path}: values differ"  # Never print private evidence.
        return None
    found = difference(left, right, label)
    if found:
        raise AssertionError(found)

import os
from pathlib import Path
from kassiber.core import custody_journal
from kassiber.core.custody_compute import STAGES, compute_custody_decisions, compute_custody_projection

MODE_ENV = "KASSIBER_TEST_CUSTODY_ENGINE"
RECORDS_ENV = "KASSIBER_TEST_CUSTODY_RECORDS"
MODULE_ENV = "KASSIBER_TEST_CUSTODY_MODULE"
INJECT_ENV = "KASSIBER_TEST_CUSTODY_INJECT_DIFFERENCE"
_ORIGINAL = None
_INSTALLED_MODE = None


def record(outcome, *, compared=(), skipped=(), reference=False):
    path = os.environ.get(RECORDS_ENV)
    if path:
        line = json.dumps({"module": os.environ.get(MODULE_ENV), "outcome": outcome, "compared": list(compared), "skipped": list(skipped), "reference": reference}) + "\n"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            os.write(fd, line.encode("utf-8"))
        finally:
            os.close(fd)


def read_records(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def compare_rust(snapshot, evidence):
    import kassiber_tax
    if not hasattr(kassiber_tax, "custody"):
        return (), STAGES
    response = json.loads(kassiber_tax.custody(json.dumps(snapshot, ensure_ascii=True, allow_nan=True)))
    if "error" in response:
        raise AssertionError("Rust custody refused the reference snapshot: " + response["error"]["code"])
    if response.get("schema_version") != 1:
        raise AssertionError("Rust custody response schema differs")
    results = response["stages"]
    skipped = response["skipped_stages"]
    if set(results) & set(skipped) or set(results) | set(skipped) != set(STAGES):
        raise AssertionError("Rust custody stage inventory differs")
    if os.environ.get(INJECT_ENV):
        results["evidence"] = {"injected": True}
    if "evidence" in results:
        assert_same(evidence, results["evidence"], "evidence")
    return tuple(results), tuple(skipped)


def _build(self, *, finalized):
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    factory = custody_journal._build_observer_factory
    observer = factory(self) if factory is not None else None
    compared, skipped, recorded = (), STAGES, False
    expected = expected_error = None
    snapshot = None
    def emit(name, value):
        nonlocal compared, skipped, recorded
        if observer is not None:
            observer.stage(name, value)
        if name == "evidence":
            try:
                compared, skipped = compare_rust(snapshot, value)
            except BaseException:
                record("difference", compared=("evidence",), skipped=(), reference=False)
                recorded = True
                raise
    try:
        snapshot = self.load_snapshot()
        if observer is not None:
            observer.stage("snapshot", snapshot)
        # The frozen flow independently uses SQL, including all historical
        # query distinctions. Actual computation below uses only this request.
        reference = ReferenceBuilder(self.conn, self.profile, as_of=snapshot["as_of"])
        try:
            expected = reference.build_custody_projection() if finalized else reference.build_custody_decisions()
        except Exception as exc:
            expected_error = exc
        compute = compute_custody_projection if finalized else compute_custody_decisions
        result = compute(snapshot, stage_observer=emit, owned_index_observer=observer.owned_index if observer is not None else None)
        if expected_error is not None:
            raise AssertionError("snapshot custody succeeded where the frozen flow failed")
        assert_same(expected, result, "Python snapshot/reference")
    except BaseException as exc:
        if observer is not None:
            observer.fail(exc)
        matched_error = expected_error is not None and type(exc) is type(expected_error) and str(exc) == str(expected_error)
        if not recorded:
            record("error" if matched_error or snapshot is None else "difference", compared=compared, skipped=skipped, reference=matched_error)
        raise
    if observer is not None:
        observer.finish(result)
    record("result", compared=compared, skipped=skipped, reference=True)
    return result


def install_custody_mode(mode):
    global _ORIGINAL, _INSTALLED_MODE
    if mode not in {"shadow", "census"}:
        raise ValueError(f"unknown custody mode: {mode!r}")
    if _INSTALLED_MODE == mode:
        return
    if _ORIGINAL is None:
        _ORIGINAL = custody_journal.CustodyJournalBuilder._compute_snapshot
    custody_journal.CustodyJournalBuilder._compute_snapshot = _build
    _INSTALLED_MODE = mode


def uninstall_custody_mode():
    global _ORIGINAL, _INSTALLED_MODE
    if _ORIGINAL is not None:
        custody_journal.CustodyJournalBuilder._compute_snapshot = _ORIGINAL
    _ORIGINAL = _INSTALLED_MODE = None


def session_report(records):
    counts = {name: sum(name in row["compared"] for row in records) for name in STAGES}
    skipped = {name: sum(name in row["skipped"] for row in records) for name in STAGES}
    lines = [f"custody snapshots: {len(records)}; frozen Python comparisons: {sum(row['reference'] for row in records)}"]
    lines.extend(f"{name}: {counts[name]} compared, {skipped[name]} skipped" for name in STAGES)
    failures = [row for row in records if row["outcome"] == "difference"]
    return lines, ([f"custody differences: {len(failures)}"] if failures else [])

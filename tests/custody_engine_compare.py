"""R2 stage comparisons. Test-only; product code never imports this module."""
from __future__ import annotations
from copy import deepcopy
from dataclasses import replace
from contextlib import contextmanager
from contextvars import ContextVar
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
    return clean(to_jsonable(value, redact_config=False))


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

import importlib
import inspect
import os
import sys
from pathlib import Path
from kassiber.core import custody_journal
from kassiber import tax_policy
from kassiber.core.custody_compute import STAGES as CUSTODY_STAGES, compute_custody_decisions, compute_custody_projection
from tests.custody_fusion_stage import add_fusion

STAGES = (*CUSTODY_STAGES, "fusion")

MODE_ENV = "KASSIBER_TEST_CUSTODY_ENGINE"
RECORDS_ENV = "KASSIBER_TEST_CUSTODY_RECORDS"
MODULE_ENV = "KASSIBER_TEST_CUSTODY_MODULE"
INJECT_ENV = "KASSIBER_TEST_CUSTODY_INJECT_DIFFERENCE"
_ORIGINAL = None
_INSTALLED_MODE = None
_INJECTED_FAILURE = ContextVar("custody_injected_failure", default=None)
_PRODUCTION_POLICY_BUILDERS = {
    tax_policy.DEFAULT_TAX_COUNTRY: tax_policy.build_generic_policy,
    tax_policy.AUSTRIAN_TAX_COUNTRY: tax_policy.build_austrian_policy,
}
PATCHED_POLICY_REASON = "patched POLICY_BUILDERS: Rust fusion uses production profile policy"
PATCHED_COMPUTE_PREFIX = "patched custody compute: "
# Modules whose functions run after the snapshot is loaded. A test that
# replaces one of them exercises Python-only behavior, so Rust, which never
# sees the patch, cannot be compared. Loader, store and authoring modules are
# absent on purpose: patching them changes the snapshot both engines receive.
_COMPUTE_MODULES = (
    "kassiber.core.custody_compute",
    "kassiber.core.custody_components",
    "kassiber.core.custody_component_claims",
    "kassiber.core.custody_component_facts",
    "kassiber.core.custody_evidence",
    "kassiber.core.custody_interpreters",
    "kassiber.core.custody_gaps",
    "kassiber.core.custody_gap_holds",
    "kassiber.core.custody_native_audit",
    "kassiber.core.custody_native_reconciliation",
    "kassiber.core.custody_native_transitions",
    "kassiber.core.custody_quantity",
    "kassiber.core.custody_quantity_runtime",
    "kassiber.core.custody_review_terms",
    "kassiber.core.custody_pricing",
    "kassiber.core.custody_tax_projection",
)
_COMPUTE_FUNCTIONS = {}


def _capture_compute_functions():
    captured = {}
    for name in _COMPUTE_MODULES:
        module = importlib.import_module(name)
        for attr, value in vars(module).items():
            if inspect.isfunction(value) and value.__module__ == name:
                captured[(name, attr)] = value
    return captured


def patched_compute_functions():
    """Name every captured custody compute function a test has replaced."""
    return sorted(
        f"{module}.{attr}"
        for (module, attr), function in _COMPUTE_FUNCTIONS.items()
        if getattr(sys.modules[module], attr, None) is not function
    )


_INJECTED_POLICY = ContextVar("custody_injected_policy", default=None)


@contextmanager
def injected_fusion_policy(builder):
    """Compare custody, except fusion under this exact test-only policy.

    Rust has the shipped policies, not Python's patched POLICY_BUILDERS.
    The opt-out is local to this context and the matching builder identity.
    """
    token = _INJECTED_POLICY.set(builder)
    try:
        yield
    finally:
        _INJECTED_POLICY.reset(token)


@contextmanager
def injected_stage_failure(stage, error):
    """Opt one intentional test failure out of parity, by stage and identity.

    This is test plumbing only. Equal messages, other exceptions, other stages
    and later builds still undergo ordinary refusal comparison.
    """
    if stage not in STAGES or not isinstance(error, BaseException):
        raise ValueError("an injected custody failure requires a stage and exception")
    token = _INJECTED_FAILURE.set((stage, error))
    try:
        yield
    finally:
        _INJECTED_FAILURE.reset(token)


def record(outcome, *, compared=(), skipped=(), reference=False, reason=None, refusing_stage=None):
    path = os.environ.get(RECORDS_ENV)
    if path:
        row = {"module": os.environ.get(MODULE_ENV), "outcome": outcome, "compared": list(compared), "skipped": list(skipped), "reference": reference}
        if refusing_stage is not None:
            row["refusing_stage"] = refusing_stage
        if reason is not None:
            row["reason"] = reason
        line = json.dumps(row) + "\n"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            os.write(fd, line.encode("utf-8"))
        finally:
            os.close(fd)


def read_records(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def python_refusal(snapshot, *, finalized=True):
    """Identify the first uncompleted stage independently of Rust's envelope."""
    observed = {}
    compute = compute_custody_projection if finalized else compute_custody_decisions
    try:
        compute(snapshot, stage_observer=lambda name, value: observed.setdefault(name, value))
    except Exception as error:
        stage = next((stage for stage in STAGES if stage not in observed), None)
        return getattr(error, "code", type(error).__name__), stage
    raise AssertionError("Python refusal could not be reproduced")


_BYTE_TAG, _DICT_TAG = "$kassiber_bytes", "$kassiber_dict"


def _carried_by_v1(snapshot):
    """Whether the strict v1 JSON adapter carries this snapshot: no SQLite
    BLOB anywhere, and every transaction quantity an integer or null."""
    def no_bytes(value):
        if isinstance(value, (bytes, bytearray, memoryview)):
            return False
        if isinstance(value, Mapping):
            return all(no_bytes(item) for item in value.values())
        if isinstance(value, (list, tuple)):
            return all(no_bytes(item) for item in value)
        return True
    quantities = all(
        row.get(key) is None or (isinstance(row.get(key), int) and not isinstance(row.get(key), bool))
        for row in snapshot.get("transactions", ()) for key in ("amount", "fee")
    )
    return quantities and no_bytes(snapshot)


def _loaded_encoding(value):
    """The database loader's record encoding (`codec::to_json`): a BLOB is the
    private byte tag, and a mapping whose sole key is a tag is escaped."""
    if isinstance(value, (bytes, bytearray, memoryview)):
        return {_BYTE_TAG: bytes(value).hex()}
    if isinstance(value, Mapping):
        encoded = {key: _loaded_encoding(item) for key, item in value.items()}
        if len(encoded) == 1 and next(iter(encoded)) in (_BYTE_TAG, _DICT_TAG):
            return {_DICT_TAG: encoded}
        return encoded
    if isinstance(value, (list, tuple)):
        return [_loaded_encoding(item) for item in value]
    return value


def _loaded_decoding(value):
    if isinstance(value, Mapping):
        if len(value) == 1 and isinstance(value.get(_BYTE_TAG), str):
            return bytes.fromhex(value[_BYTE_TAG])
        if len(value) == 1 and isinstance(value.get(_DICT_TAG), Mapping):
            return {key: _loaded_decoding(item) for key, item in value[_DICT_TAG].items()}
        return {key: _loaded_decoding(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_loaded_decoding(item) for item in value]
    return value


def rust_results(snapshot, *, python_error=None, python_stage=None, finalized=True, finalized_refusals=None):
    import kassiber_tax
    if not hasattr(kassiber_tax, "custody"):
        raise AssertionError("Rust custody binding is missing")
    # Journal builds run the loaded path, which keeps a book's legacy SQLite
    # scalars. The strict v1 adapter refuses them, so such a book is compared
    # through the loader's own encoding instead.
    if _carried_by_v1(snapshot):
        response = json.loads(kassiber_tax.custody(json.dumps(snapshot, ensure_ascii=True, allow_nan=True), finalized=finalized))
    else:
        request = json.dumps(_loaded_encoding(snapshot), ensure_ascii=True, allow_nan=True)
        response = _loaded_decoding(json.loads(kassiber_tax.custody(request, finalized=finalized, loaded=True)))
    if python_error is not None:
        code = getattr(python_error, "code", type(python_error).__name__)
        if response.get("error", {}).get("code") != code:
            raise AssertionError("Rust custody refusal code differs from Python: " + code)
        if python_stage is None:
            reproduced_code, python_stage = python_refusal(snapshot, finalized=finalized)
            if reproduced_code != code:
                raise AssertionError("Python refusal changed during replay")
        if response.get("error", {}).get("stage") != python_stage:
            raise AssertionError("Rust custody refusal stage differs from Python: " + str(python_stage))
        return {}, STAGES
    if "error" in response:
        if finalized and finalized_refusals is not None:
            code, stage = python_refusal(snapshot)
            if (response["error"].get("code"), response["error"].get("stage")) != (code, stage):
                raise AssertionError("Rust custody finalized refusal code/stage differs from Python")
            if stage != "finalized_projection":
                raise AssertionError("earlier-stage comparison cannot bypass a refusal at " + str(stage))
            finalized_refusals[stage + ":" + code] += 1
            return rust_results(snapshot, finalized=False)
        raise AssertionError("Rust custody refused the reference snapshot: " + response["error"]["code"])
    if response.get("schema_version") != 1:
        raise AssertionError("Rust custody response schema differs")
    results = response["stages"]
    if not results:
        raise AssertionError("Rust custody binding compared no stages")
    skipped = response["skipped_stages"]
    if set(results) & set(skipped) or set(results) | set(skipped) != set(STAGES):
        raise AssertionError("Rust custody stage inventory differs")
    if os.environ.get(INJECT_ENV):
        stage = os.environ[INJECT_ENV]
        stage = stage if stage in STAGES else next(iter(results), "evidence")
        results[stage] = {"injected": True}
    return results, tuple(skipped)


def count_valuation_holds(snapshot, results, holds):
    """Count a book whose finalized projection holds an invalid price, and
    compare that projection with Python's.

    Since round 5 an unusable stored price is an invalid_transaction_price
    hold, not a finalized-stage refusal, so the held books' whole projection
    is compared here.
    """
    projection = results.get("finalized_projection")
    if projection is None or not any(
            q.get("reason") == "invalid_transaction_price" for q in projection.get("quarantines", [])):
        return
    from tests.custody_finalized_stage import python_stages
    assert_same(to_jsonable(python_stages(snapshot)["finalized_projection"], redact_config=False),
                projection, "finalized_projection")
    holds["finalized_projection:invalid_transaction_price"] += 1


def compare_rust(snapshot, evidence, *, python_stages=None):
    if python_stages is None:
        stages = {}
        compute_custody_projection(snapshot, stage_observer=lambda name, value: stages.setdefault(name, value))
        stages["evidence"] = evidence
    else:
        stages = python_stages
    stages = add_fusion(snapshot, dict(stages))
    results, skipped = rust_results(snapshot, finalized="finalized_projection" in stages)
    compared = []
    for name in STAGES:
        if name in results and name in stages:
            assert_same(stages[name], results[name], name)
            compared.append(name)
    return tuple(compared), tuple(skipped)


def permuted_snapshot(snapshot):
    """Reverse unordered SQL reads while preserving ordered rows and rate ties."""
    result = deepcopy(snapshot)
    for field in ("wallets", "loan_legs", "channel_records", "pair_dismissals", "receipt_sources", "components", "anchor_wallets", "revision_headers"):
        result[field].reverse()
    result["anchors"] = dict(reversed(tuple(result["anchors"].items())))
    index = result.get("owned_index")
    if index is not None:
        for field in ("by_script", "by_outpoint"):
            for matches in index[field].values():
                matches.reverse()
        for field in ("by_scoped_outpoint", "txid_wallets_by_scope"):
            for row in index[field]:
                row["matches"].reverse()
        for matches in index["txid_wallets"].values():
            matches.reverse()
    return result


def _build(self, *, finalized):
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    factory = custody_journal._build_observer_factory
    observer = factory(self) if factory is not None else None
    compared, skipped, recorded = (), STAGES, False
    expected = expected_error = None
    audit_reference = _INSTALLED_MODE == "shadow"
    snapshot = None
    python_stages = {}
    patched_policy = tax_policy.POLICY_BUILDERS != _PRODUCTION_POLICY_BUILDERS
    skip_reason = PATCHED_POLICY_REASON if patched_policy and finalized else None
    patched_compute = patched_compute_functions()
    if patched_compute:
        audit_reference = False
    def emit(name, value):
        if observer is not None:
            observer.stage(name, value)
        python_stages[name] = value

    def compare_outcome(error=None):
        nonlocal compared, skipped, recorded
        failed = next((stage for stage in STAGES if stage not in python_stages), None)
        injection = _INJECTED_FAILURE.get()
        if injection is not None and injection[0] == failed and injection[1] is error:
            compared, skipped = (), STAGES
            record("injected", skipped=skipped, reason="injected", refusing_stage=failed)
            recorded = True
            return
        if patched_compute:
            compared, skipped = (), STAGES
            record("patched", skipped=skipped, reason=PATCHED_COMPUTE_PREFIX + ", ".join(patched_compute), refusing_stage=failed)
            recorded = True
            return
        try:
            # The binding returns an atomic refusal, without earlier outputs.
            # Compare its code at the failed implemented stage, and explicitly
            # skip unavailable outputs, including decision-only finalization.
            if error is not None and failed in STAGES:
                compared = (failed,)
                skipped = tuple(stage for stage in STAGES if stage != failed)
                rust_results(snapshot, python_error=error, python_stage=failed, finalized=finalized)
            else:
                results, skipped = rust_results(snapshot, finalized=finalized)
                if skip_reason:
                    results.pop("fusion", None)
                    skipped = tuple(stage for stage in STAGES if stage in skipped or stage == "fusion")
                from kassiber.tax_policy import POLICY_BUILDERS
                policy = _INJECTED_POLICY.get()
                if "fusion" in results and policy is not None and POLICY_BUILDERS.get(snapshot["profile"].get("tax_country")) is policy:
                    results.pop("fusion", None)
                    skipped = (*skipped, "fusion")
                for stage in STAGES:
                    if stage in results:
                        assert stage in python_stages, "Python omitted implemented stage: " + stage
                        compared = (*compared, stage)
                        assert_same(python_stages[stage], results[stage], stage)
            assert not set(compared) & set(skipped)
            assert set(compared) | set(skipped) == set(STAGES)
        except BaseException:
            record("difference", compared=compared, skipped=skipped)
            recorded = True
            raise

    try:
        snapshot = self.load_snapshot()
        from tests.rust_port.custody_loader_oracle import compare as compare_loader
        try:
            compare_loader(self.conn, self.profile, snapshot)
        except BaseException:
            # A test can intentionally catch an AssertionError. Keep loader
            # differences fatal to the session, like custody-stage differences.
            record("difference", compared=(), skipped=STAGES)
            recorded = True
            raise
        if observer is not None:
            observer.stage("snapshot", snapshot)
        # Census preserves the one-build read budget; shadow also audits SQL.
        if audit_reference:
            reference = ReferenceBuilder(self.conn, self.profile, as_of=snapshot["as_of"])
            try:
                expected = reference.build_custody_projection() if finalized else reference.build_custody_decisions()
            except Exception as exc:
                expected_error = exc
        compute = compute_custody_projection if finalized else compute_custody_decisions
        try:
            result = compute(snapshot, stage_observer=emit, owned_index_observer=observer.owned_index if observer is not None else None)
            if finalized and not skip_reason:
                from tests.custody_fusion_stage import python_fusion
                emit("fusion", python_fusion(snapshot, python_stages["finalized_projection"]))
        except BaseException as exc:
            compare_outcome(exc)
            raise
        compare_outcome()
        if expected_error is not None:
            raise AssertionError("snapshot custody succeeded where the frozen flow failed")
        if audit_reference:
            expected = normalized_reference(expected)
            assert_same(expected, result, "Python snapshot/reference")
    except BaseException as exc:
        if observer is not None:
            observer.fail(exc)
        matched_error = expected_error is not None and type(exc) is type(expected_error) and str(exc) == str(expected_error)
        if not recorded:
            record("error" if not audit_reference or matched_error or snapshot is None else "difference", compared=compared, skipped=skipped, reference=matched_error, reason=skip_reason, refusing_stage=next((s for s in STAGES if s not in python_stages), None))
        raise
    if observer is not None:
        observer.finish(result)
    if not recorded:
        record("result", compared=compared, skipped=skipped, reference=audit_reference,
               reason="injected_policy" if "fusion" in skipped and _INJECTED_POLICY.get() is not None else skip_reason)
    return result


def install_custody_mode(mode):
    global _ORIGINAL, _INSTALLED_MODE
    if mode not in {"shadow", "census"}:
        raise ValueError(f"unknown custody mode: {mode!r}")
    if _INSTALLED_MODE == mode:
        return
    if _ORIGINAL is None:
        _ORIGINAL = custody_journal.CustodyJournalBuilder._compute_snapshot
    _COMPUTE_FUNCTIONS.clear()
    _COMPUTE_FUNCTIONS.update(_capture_compute_functions())
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
    injected = [row for row in records if row["outcome"] == "injected"]
    lines.append(f"injected builds: {len(injected)} (all stages skipped)")
    patched = [row for row in records if row.get("reason") == PATCHED_POLICY_REASON]
    lines.append(f"patched-policy builds: {len(patched)} (fusion skipped)")
    patched_compute = [row for row in records if row["outcome"] == "patched"]
    lines.append(f"patched-compute builds: {len(patched_compute)} (all stages skipped)")
    failures = [row for row in records if row["outcome"] == "difference"]
    empty = [row for row in records if row["outcome"] == "result" and not row["compared"]]
    errors = ([f"custody differences: {len(failures)}"] if failures else [])
    incomplete = [row for row in records if row["outcome"] in {"result", "error", "injected", "patched"} and
                  (set(row["compared"]) & set(row["skipped"]) or
                   set(row["compared"]) | set(row["skipped"]) != set(STAGES))]
    if incomplete:
        errors.append("custody stage inventory is incomplete")
    if any(row.get("reason") != "injected" or row["compared"] or set(row["skipped"]) != set(STAGES) for row in injected):
        errors.append("custody injected skip record is invalid")
    if any("fusion" in row["compared"] or "fusion" not in row["skipped"] for row in patched):
        errors.append("custody patched-policy skip record is invalid")
    if any(not str(row.get("reason", "")).startswith(PATCHED_COMPUTE_PREFIX) or row["compared"]
           or set(row["skipped"]) != set(STAGES) for row in patched_compute):
        errors.append("custody patched-compute skip record is invalid")
    if empty:
        errors.append("custody binding compared no stages")
    return lines, errors


def normalized_reference(value):
    """Apply the explicit secret-free request boundary to the frozen output only.

    Actual config text is compared exactly; normalization must not hide a bug
    in the product's loader or later copies.
    """
    from kassiber.core.custody_snapshot import normalized_config
    value = comparable(value)
    def walk(item):
        if isinstance(item, dict):
            return {key: normalized_config(val) if key in {"config_json", "wallet_config_json"} else walk(val) for key, val in item.items()}
        if isinstance(item, list):
            return [walk(val) for val in item]
        return item
    return walk(value)

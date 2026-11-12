"""Production and the live oracle against the projection-level goldens (plan 20).

``tax-engine/core/tests/data/ledger_fixtures.json`` stores, per case, a
profile and finalized projection, the frozen oracle's outcome on RP2, and the
outcome production must give (``tax-engine/scripts/gen_ledger_fixtures.py``
writes and certifies it). Here:

- production on the stored inputs gives ``expected`` exactly;
- the live oracle on the stored inputs still gives the stored ``oracle``. The
  oracle shares product modules with production (``tests/tax_oracle/README.md``),
  so a change in one of them moves both and only this check sees it;
- the file is the generator's corpus, ``expected`` follows from ``oracle``
  by the listed differences alone (an LD-5 case's ``expected`` is the request
  builder's refusal of its one out-of-range shape), two fresh generator runs
  write the file byte for byte, and the generator refuses to change a stored
  case without ``--accept``;
- the random half: 200 seeded generic and 200 Austrian books of
  ``tests/tax_ledger_generator.py``, oracle against production. Every third
  book uses the generator's ``terms``: a generic holding period other than
  365 days, or an Austrian method beyond the three the corpus seeds draw.

The oracle needs RP2, a development dependency. Without it the oracle checks
skip, unless ``KASSIBER_CI`` is set, where they fail.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from functools import lru_cache
from importlib.util import find_spec
from pathlib import Path
from typing import Any

import pytest

from tests import tax_engine_compare as compare
from tests import tax_ledger_codec as codec
from tests.tax_ledger_generator import corpus, random_inputs
from tests.tax_oracle import projection_range

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"
GENERATOR = ROOT / "tax-engine" / "scripts" / "gen_ledger_fixtures.py"
RANDOM_SEEDS = 200
MAX_REPORTED = 5


@lru_cache(maxsize=1)
def _document() -> dict[str, Any]:
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _case_names() -> list[str]:
    return [case["name"] for case in _document()["cases"]]


def _case(name: str) -> dict[str, Any]:
    return next(case for case in _document()["cases"] if case["name"] == name)


def _require_oracle() -> None:
    if find_spec("rp2") is not None:
        return
    if os.environ.get("KASSIBER_CI"):
        pytest.fail("RP2 is missing, so the oracle cannot run; CI installs it with the dev dependency group")
    pytest.skip("RP2 (a development dependency) is not installed")


def test_without_rp2_the_oracle_checks_skip_except_in_ci(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys.modules[__name__], "find_spec", lambda name: None)
    monkeypatch.delenv("KASSIBER_CI", raising=False)
    with pytest.raises(pytest.skip.Exception):
        _require_oracle()
    monkeypatch.setenv("KASSIBER_CI", "1")
    with pytest.raises(pytest.fail.Exception):
        _require_oracle()


@pytest.mark.parametrize("name", _case_names())
def test_production_gives_the_expected_outcome(name: str) -> None:
    case = _case(name)
    profile, inputs = codec.decode_inputs(case["inputs"])
    actual = compare.outcome_document(compare.run_production(profile, inputs))
    assert compare.document_differences(codec.decode(case["expected"]), actual) == []


@pytest.mark.parametrize("name", _case_names())
def test_the_live_oracle_gives_the_stored_outcome(name: str) -> None:
    _require_oracle()
    case = _case(name)
    profile, inputs = codec.decode_inputs(case["inputs"])
    actual = compare.outcome_document(compare.run_oracle(profile, inputs), compare.oracle_error_signature)
    assert compare.document_differences(codec.decode(case["oracle"]), actual) == []


def test_expected_follows_from_the_oracle_by_the_listed_differences() -> None:
    for case in _document()["cases"]:
        oracle, expected = codec.decode(case["oracle"]), codec.decode(case["expected"])
        difference = case["listed_difference"]
        if difference is None:
            assert compare.document_differences(oracle, expected) == [], case["name"]
            continue
        if difference == "LD-5":
            # The builder's refusal names the case's invariant, which the
            # inputs have; the oracle's outcome is not compared.
            name, message = expected["error"]
            assert name == "EngineContractError", case["name"]
            assert message.startswith(f"RequestError: {case['invariant']}: "), case["name"]
            profile, inputs = codec.decode_inputs(case["inputs"])
            assert case["invariant"] in projection_range.violations(profile, inputs), case["name"]
            assert case["request"] is None, case["name"]
            continue
        assert difference in ("LD-3", "LD-4"), case["name"]
        assert case["invariant"] is None, case["name"]
        assert "error" in oracle and "error" in expected, case["name"]
        assert compare.listed_signature(oracle["error"]) == expected["error"], case["name"]
        assert oracle["error"] != expected["error"], case["name"]
        prefixed = compare.STORED_INI_PATH in json.dumps(case["oracle"])
        assert prefixed == (difference == "LD-3"), case["name"]


def test_every_stored_projection_is_in_range() -> None:
    # Except the LD-5 cases, which have exactly their one shape.
    invariants = set()
    for case in _document()["cases"]:
        profile, inputs = codec.decode_inputs(case["inputs"])
        expected = (case["invariant"],) if case["invariant"] else ()
        assert projection_range.violations(profile, inputs) == expected, case["name"]
        assert (case["listed_difference"] == "LD-5") == bool(case["invariant"]), case["name"]
        invariants.add(case["invariant"])
    # Every invariant but the pool scope, which raises the oracle's own error.
    assert invariants - {None} == set(projection_range.INVARIANTS) - {projection_range.NON_GLOBAL_POOL_SCOPE}


def test_the_goldens_are_the_generators_corpus() -> None:
    # A corpus change, or a change in the custody stages that build its
    # projections, needs a generator run, which certifies it.
    cases = corpus()
    assert _case_names() == [case.name for case in cases]
    for case in cases:
        stored = _case(case.name)
        assert stored["tags"] == list(case.tags), case.name
        assert stored["invariant"] == case.invariant, case.name
        assert stored["inputs"] == json.loads(json.dumps(codec.encode_inputs(case.profile, case.inputs))), case.name


def test_the_codec_keeps_what_the_comparison_checks() -> None:
    from decimal import Decimal

    from kassiber.core.transfer_chronology import ChainOrderEvidence

    scope = ("bitcoin", "main", "a" * 64, "BTC")
    value = {
        "decimals": [Decimal("1.0"), Decimal("-0"), Decimal("1E+2"), Decimal("NaN")],
        "floats": [0.1, -0.0, float("inf"), 1e16],
        "tuple": (1, "x", None, True),
        "keys": {("w", "Cold", "treasury", "BTC"): {"quantity": Decimal("0.5")}},
        "$tag-like": {"$decimal": "1"},
        "evidence": ChainOrderEvidence(scope, frozenset({scope[:2] + ("b" * 64, "BTC"), scope})),
        "set": frozenset({"b", "a"}),
    }
    stored = json.loads(json.dumps(codec.encode(value)))
    decoded = codec.decode(stored)
    assert compare.value_differences(value, decoded) == []
    assert [type(item) for item in decoded["decimals"]] == [Decimal] * 4
    assert decoded["decimals"][3].is_nan()
    assert decoded["evidence"] == value["evidence"]
    assert decoded["set"] == value["set"]
    assert list(decoded) == list(value)
    with pytest.raises(TypeError):
        codec.encode(object())


def _generate(output: Path, *accept: str, hash_seed: str = "0") -> subprocess.CompletedProcess:
    command = [sys.executable, str(GENERATOR), "--output", str(output)]
    for name in accept:
        command += ["--accept", name]
    env = {**os.environ, "PYTHONHASHSEED": hash_seed}
    return subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, timeout=600)


def test_two_generator_runs_reproduce_the_stored_file(tmp_path: Path) -> None:
    # Fresh runs (nothing stored, so no --accept) under different string
    # hashing write the stored file byte for byte.
    _require_oracle()
    for hash_seed in ("0", "1"):
        output = tmp_path / f"run-{hash_seed}.json"
        run = _generate(output, hash_seed=hash_seed)
        assert run.returncode == 0, run.stderr
        assert output.read_bytes() == FIXTURES.read_bytes(), f"PYTHONHASHSEED={hash_seed}"


def test_the_generator_refuses_to_change_a_stored_outcome(tmp_path: Path) -> None:
    _require_oracle()
    stored = tmp_path / "ledger_fixtures.json"
    document = json.loads(FIXTURES.read_text(encoding="utf-8"))
    cases = {case["name"]: case for case in document["cases"]}
    # One stored value with another exponent: equal as a number, not as an outcome.
    cases["generic-ledger-fifo"]["oracle"]["result"]["entries"][0]["quantity"] = {"$decimal": "1.00"}
    # Inputs the corpus no longer builds, with the outcome left alone.
    cases["generic-unpriced"]["inputs"]["profile"]["label"] = "Renamed"
    # An expected outcome or a listed difference the oracle no longer implies.
    cases["at-seed-3"]["expected"]["result"]["entries"][0]["quantity"] = {"$decimal": "0.917987406"}
    cases["at-rail-fifo"]["listed_difference"] = None
    # A case the corpus no longer has.
    document["cases"].append({**cases["generic-gift-lost"], "name": "retired-case"})
    stored.write_text(json.dumps(document, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    tampered = stored.read_bytes()

    refused = _generate(stored)
    assert refused.returncode != 0
    for complaint in (
        "generic-ledger-fifo: oracle would change",
        "generic-unpriced: inputs would change",
        "at-seed-3: expected would change",
        "at-rail-fifo: listed_difference would change",
        "retired-case: the case is no longer generated",
    ):
        assert complaint in refused.stderr
    assert stored.read_bytes() == tampered

    partly = _generate(stored, "generic-ledger-fifo", "generic-unpriced", "at-seed-3", "at-rail-fifo")
    assert partly.returncode != 0
    assert "retired-case" in partly.stderr
    for name in ("generic-ledger-fifo", "generic-unpriced", "at-seed-3", "at-rail-fifo"):
        assert name not in partly.stderr
    assert stored.read_bytes() == tampered

    accepted = _generate(stored, "generic-ledger-fifo", "generic-unpriced", "at-seed-3", "at-rail-fifo", "retired-case")
    assert accepted.returncode == 0, accepted.stderr
    assert stored.read_bytes() == FIXTURES.read_bytes()


@pytest.mark.parametrize("country", ["generic", "at"])
def test_random_books_match_the_oracle(country: str) -> None:
    _require_oracle()
    failures = []
    for seed in range(RANDOM_SEEDS):
        book = random_inputs(seed, country, terms=seed % 3 == 2)
        violations = projection_range.violations(book.profile, book.inputs)
        if violations:
            failures.append(f"seed {seed}: out of range {violations}")
            continue
        differences = compare.ledger_differences(
            compare.run_oracle(book.profile, book.inputs),
            compare.run_production(book.profile, book.inputs),
        )
        if differences:
            failures.append(f"seed {seed} {book.tags}:\n    " + "\n    ".join(differences[:10]))
    assert not failures, f"{len(failures)} of {RANDOM_SEEDS} {country} books differ:\n" + "\n".join(failures[:MAX_REPORTED])

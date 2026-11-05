"""Write Python's secret-floor output for the Rust scanner's parity tests.

`fixtures` writes tests/data/secret_floor.txt: one JSON object per line,
`{"set", "in", "out"}`, for every case of the shared fixture corpora
(tests/fixtures/secret_redaction_cases.json and the 5e to 5i rounds), with
`out = redact_secret_text(in)` from kassiber/redaction.py at this checkout.

`generators DIR` writes the six reviewer generators' cases, with Python's
output, as DIR/<generator>-<seed>.jsonl, and DIR/manifest.json. Pinned seeds
must reproduce the pinned output digest; each generator also runs its pinned
extra seeds and two more. The files are large, so they stay out of the tree:

    .venv/bin/python crates/envelope/scripts/gen_scanner_vectors.py fixtures
    .venv/bin/python crates/envelope/scripts/gen_scanner_vectors.py generators "$(realpath target)/redaction-corpus"
    KASSIBER_REDACTION_CORPUS="$(realpath target)/redaction-corpus" \\
        cargo test --offline -p kassiber-envelope --test scanner_corpus -- --ignored

Inputs are written with `ensure_ascii=True`, so surrogates survive as escapes.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.redaction import redact_secret_text  # noqa: E402

FIXTURES = ROOT / "tests" / "fixtures"
TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "secret_floor.txt"
CORPORA = ["cases", "5e_cases", "5f_cases", "5g_cases", "5h_cases", "5i_cases"]
EXTRA_SEEDS = [1001, 1002]
EXTRA_COUNT = 20_000


def line(record: dict) -> str:
    return json.dumps(record, separators=(",", ":"), ensure_ascii=True)


def fixtures() -> None:
    lines = []
    for corpus in CORPORA:
        for case in json.loads((FIXTURES / f"secret_redaction_{corpus}.json").read_text()):
            text = case["input"]
            lines.append(line({"set": corpus, "in": text, "out": redact_secret_text(text)}))
    TARGET.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(lines)} cases to {TARGET.relative_to(ROOT)}")


def generators():
    """(name, spec, cases(count, seed)) for each reviewer generator."""
    from tests import (
        redaction_fifth_generation,
        redaction_fourth_generation,
        redaction_generation,
        redaction_independent_generation,
        redaction_sixth_generation,
        redaction_third_generation,
    )

    def wire(count, seed):
        saved = dict(redaction_generation.SPEC)
        redaction_generation.SPEC.update(count=count, seed=seed)
        try:
            yield from redaction_generation.generated_cases()
        finally:
            redaction_generation.SPEC.clear()
            redaction_generation.SPEC.update(saved)

    def spec(name):
        return json.loads((FIXTURES / f"secret_redaction_{name}_generator.json").read_text())

    yield "wire", dict(redaction_generation.SPEC), wire
    yield "independent", spec("independent"), redaction_independent_generation.generated_cases
    yield "third", spec("third"), redaction_third_generation.cases
    yield "fourth", spec("fourth"), redaction_fourth_generation.cases
    yield "fifth", spec("fifth"), redaction_fifth_generation.cases
    yield "sixth", spec("sixth"), redaction_sixth_generation.cases


def write_generators(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for name, spec, cases in generators():
        runs = [(spec["seed"], spec["count"], spec["output_sha256"])]
        runs += [(seed, spec.get("extra_count", EXTRA_COUNT), None) for seed in spec.get("extra_seeds", [])]
        runs += [(seed, EXTRA_COUNT, None) for seed in EXTRA_SEEDS]
        for seed, count, pinned in runs:
            digest = hashlib.sha256()
            path = directory / f"{name}-{seed}.jsonl"
            with path.open("w", encoding="utf-8") as handle:
                for case in cases(count, seed):
                    output = redact_secret_text(case["input"])
                    digest.update((output + "\n").encode())
                    handle.write(line({"in": case["input"], "out": output}) + "\n")
            if pinned is not None and digest.hexdigest() != pinned:
                raise SystemExit(f"{name} seed {seed}: Python no longer reproduces the pinned digest")
            manifest[path.name] = {"count": count, "pinned_sha256": pinned, "python_sha256": digest.hexdigest()}
            print(f"{path.name}: {count} cases")
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


def main() -> int:
    if sys.version_info[:2] != (3, 11):
        raise SystemExit("run with the locked Python 3.11 interpreter")
    if sys.argv[1:] == ["fixtures"]:
        fixtures()
    elif len(sys.argv) == 3 and sys.argv[1] == "generators":
        write_generators(Path(sys.argv[2]))
    else:
        raise SystemExit(__doc__)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

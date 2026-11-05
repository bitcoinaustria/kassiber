"""Write Python's secret-floor output for the Rust scanner's parity tests.

`fixtures` writes tests/data/secret_floor.txt: one JSON object per line,
`{"set", "in", "out"}`, for every case of the shared fixture corpora
(tests/fixtures/secret_redaction_cases.json and the 5e to 5i rounds), with
`out = redact_secret_text(in)` from kassiber/redaction.py at this checkout,
and tests/data/scanner_families.json: `{family: [[in, out], ...]}` for the
small per-rule inputs below.

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
FAMILY_TARGET = TARGET.with_name("scanner_families.json")
EXTRA_SEEDS = [1001, 1002]
EXTRA_COUNT = 20_000


# Small inputs per rule family; scanner_families.rs names a test per family,
# which the mutation entries in mutations.json must make fail.
FAMILIES = {
    "segment": ["rpcpassword=Pw9Leak", "dbprivkeyraw=Pw9Leak", "krakenpassword=Pw9Leak", "passwordbackup=Pw9Leak", "dnsseed=Pw9Leak", "bypass=Pw9Leak", "mytokenizer=Pw9Leak", "rpcSECRETkeyOLD=Pw9Leak", "newPWDb64=Pw9Leak", "pruneheight=12", "ftprune=Pw9Leak", "rootpwdconfirm=Pw9Leak", "db_pw=Pw9Leak", "pw_hash=abc", "author=Pw9Leak", "user_id=Pw9Leak", "sighash=abc", "api key: Pw9Leak", "\"Secret Key\": \"Pw9Leak\"", "x" * 50 + "password=Pw9Leak", "secret_v2=Pw9Leak", "2FA_code=123456", "passwordless=true", "seedwords=Pw9Leak"],
    "count": ["max_tokens: 16", "descriptors=true", "disable_private_keys=true", "\"max_tokens\": 16", "\"seed\": 42", "\"password\": null", "\"token_count\": 12, \"x\": 1", "token_limit=1.5", "\"prompt_tokens_details\": {\"cached_tokens\": 0}", "password=16", "tokens=3"],
    "literal": ["\"password\": 123, \"next\": 1", "password: 123\n  continued", "password: 123)\nnext", "--password 123 --verbose", "password=12a", "\"token\": true}", "password: 1e5 x"],
    "flag": ["bitcoind -rpcpassword Pw9Leak -daemon", "cmd --password Pw9Leak --verbose", "cmd --password correct -horse battery", "docker run -e TOKEN Pw9Leak -e NETWORK=mainnet", "kassiber --db-passphrase Pw9Leak -nodebuglogfile", "cmd --seed-words 24", "Use --password to set a value.", "cmd --password Pw9Leak\\ --next", "curl -u alice:Pw9Leak https://h"],
    "continuation": ["password: Pw9\n  Leak\nnext: 1", "macaroon: |\n  0201036c\n  Pw9Leak\nother: 1", "- password: Pw9Leak\n  - x", "token: Pw9\r\n  Leak\r\nnext", "password: a\n\n  b", "secret: {\n  a: Pw9Leak\n}\nnext", "password: \"Pw9\nLeak"],
    "url": ["https://alice:Pw9Leak@host/path", "https://alice:Pw9Leak@host", "http://u:Pw9%40Leak@h/", "https://Pw9#Leak@host", "see https://example.com/@handle:Pw9Leak today", "https://example.com/user@mail.com", "url=https://h\nalice:Pw9Leak@host", "https://a.example,http://alice:Pw9Leak@target/", "ftp://u:p@ss@word@host/", "https://example.com/path\nPw9Leak@other", "\"url\": \"http://h\", \"password\": \"Pw9Leak\"", "https://u:%40q (ZK1Pw9Leak@h", "http://[::1]:8332/x", "https://host:8332@x"],
    "path_token": ["https://api.telegram.org/bot123456:Pw9Leak/getMe", "https://hooks.slack.com/services/T00/B00/Pw9Leak", "https://discord.com/api/webhooks/123/Pw9Leak", "token xoxb-1-Pw9Leak", "xapp-1-Pw9Leak", "https://hooks.slack.com/workflows/Pw9Leak"],
    "keyed": ["password=\"Pw9Leak\" next=1", "password='''Pw9Leak''' x", "password=\"Pw9Leak\"x", "{\"password\":\"Pw9Leak\",\"ok\":1}", "Authorization: Basic Pw9Leak\n  more", "Cookie: a=Pw9Leak; b=2", "password -> Pw9Leak", "password := Pw9Leak", "pass: 123 tests ", "user: alice", "auth: ok", "password: none", "url?key=Pw9Leak&x=1", "password: $(cat secret)", "password: `Pw9Leak`"],
    "xml": ["<password>Pw9Leak</password>", "<entry key=\"password\" value=\"Pw9Leak\"/>", "<key>password</key><string>Pw9Leak</string>", "<input name=\"password\" value=\"Pw9Leak\">", "<user>alice</user>", "<ns:token>Pw9Leak</ns:token >", "<entrykey=\"password\"value=\"Pw9Leak\">", "<password>Pw9Leak", "<meta name=\"token\" content=\"Pw9Leak\">"],
    "pair": ["name=password value=Pw9Leak", "{\"name\": \"api_key\", \"value\": \"Pw9Leak\"}", "key: token, value: Pw9Leak", "name=xvalue: Pw9Leak", "name=a:password value=Pw9Leak", "\"key\":\"secret\",\"value\":'Pw9Leak'"],
    "cli": ["curl -u alice:Pw9Leak https://h", "sshpass -p Pw9Leak ssh h", "mysql -u root -pPw9Leak db", "curl -u \"alice:Pw9Leak\" x", "xcurl -u a:b"],
    "bare": ["alice:Pw9Leak@host", "mailto:alice@example.com", "password:x@h", "a:b@c:d@e", "user:pw@h https://x", "sip:alice@h"],
    "view": ["http%3A%2F%2Falice%3APw9Leak%40host", "https:\\/\\/alice:Pw9Leak@host", "http&#58;//alice:Pw9Leak&#64;host", "\\u0070assword=Pw9Leak", "ｈｔｔｐ://alice:Pw9Leak@host", "&#xD800;password=Pw9Leak", "%2570assword=Pw9Leak", "pa&#115;sword=Pw9Leak"],
    "merge": ["http://[::1]?rpcpassword=p@Leak9x failed", "http://dom;password:PartA,PartB@host:8332/", "\"HTTP://a%40b:#QJBBBBRDCJQ%2F%2F@[::1]:#f,\"password\":\"x\"", "password=x token: |\n  a\n\n  Pw9Leak\nstatus ok", "https://alice:Pw9Leak@host password=Pw9Leak2", "<password>https://u:Pw9Leak@h</password> ok", "token=Pw9Leak https://u:p@h", "name=password value=https://u:Pw9Leak@h"],
    "floor": ["xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi", "nsec1" + "q"*30, "5" + "H"*50, "K" + "w"*51, "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8", "sp(" + "a"*12 + ")", "spscan1q" + "a"*20, "tsp1q" + "a"*30, "wpkh(" + "a"*20 + ")", "combo(" + "b"*16 + ")", "Bearer abc.def==", "sk-abcdefgh", "sk-abcdef.x_", "sk-abc.def", "Bearer tok", "xsk-abcdefgh"],
}


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
    families = {name: [[text, redact_secret_text(text)] for text in texts] for name, texts in FAMILIES.items()}
    FAMILY_TARGET.write_text(json.dumps(families, ensure_ascii=True, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {sum(map(len, families.values()))} family cases to {FAMILY_TARGET.relative_to(ROOT)}")


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

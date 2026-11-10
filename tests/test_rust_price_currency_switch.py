"""A book whose currency changes holds its old prices on both runtimes.

The same EUR book is built on the Python reference and on the Rust binary,
then switched to a book of another currency. Every envelope, the stored
profile, transactions and quarantines, and the reports must be the same
bytes once generated ids and wall-clock times are masked: the priced rows
are held as `price_currency_mismatch`, never reported in the new currency
(docs/reference/tax.md, Quarantines). Python supports `generic` and `at`
only, so `--fiat-currency USD` and a USD book switched to `at` are compared;
the Rust-only countries (US, GB, CH, JP, each with its own currency) must
hold the same rows with the same envelope and detail.
"""
from __future__ import annotations

import json
import os
import re
import sqlite3
import subprocess
from pathlib import Path

import pytest

from tests import cli_runner

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")

BOOK_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2024-01-10T12:00:00Z,buy-1,inbound,BTC,1.00000000,0,40000,Buy
2024-06-10T12:00:00Z,sell-1,outbound,BTC,0.50000000,0,60000,Sell
"""
SCOPE = ("--workspace", "Main", "--profile", "Book")
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{32}")
# A wall-clock instant: any timestamp but the book's own dates.
INSTANT = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)")
BOOK_DATES = {"2024-01-10T12:00:00Z", "2024-06-10T12:00:00Z"}
STORED = {
    "profiles": "SELECT label, fiat_currency, tax_country, tax_long_term_days, gains_algorithm,"
    " cost_basis_pool_scope, journal_semantics_version, required_journal_semantics_version"
    " FROM profiles ORDER BY label",
    "transactions": "SELECT external_id, fiat_currency, fiat_rate, fiat_rate_exact, fiat_value,"
    " fiat_value_exact, pricing_source_kind, pricing_quality FROM transactions ORDER BY occurred_at",
    "journal_quarantines": "SELECT t.external_id, q.reason, q.detail_json FROM journal_quarantines q"
    " JOIN transactions t ON t.id = q.transaction_id ORDER BY t.occurred_at",
    "journal_entries": "SELECT COUNT(*) FROM journal_entries",
}
REPORTS = (
    ("reports", "tax-summary"),
    ("reports", "capital-gains"),
    ("reports", "summary"),
    ("reports", "balance-sheet"),
    ("journals", "quarantined"),
)


def _env(tmp_path: Path) -> dict[str, str]:
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700, exist_ok=True)
    # Children must not resolve the owner's real app-data root.
    env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
    env.update({
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "XDG_CONFIG_HOME": str(tmp_path / "config"),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_STATE_HOME": str(tmp_path / "state"),
    })
    return env


class Runtime:
    def __init__(self, name: str, tmp_path: Path):
        self.name = name
        self.root = tmp_path / name / "data"
        self.env = _env(tmp_path)
        self.csv = tmp_path / "book.csv"
        self.csv.write_text(BOOK_CSV, encoding="utf-8")

    def run(self, *args: str) -> str:
        argv = ["--machine", "--data-root", str(self.root), *args]
        if self.name == "rust":
            result = subprocess.run([RUST_BIN, *argv], env=self.env, capture_output=True,
                                    text=True, timeout=300, check=False)
        else:
            result = cli_runner.run(argv, reference=True, env=self.env, capture_output=True,
                                    text=True, timeout=300, check=False)
        assert result.stdout, result.stderr
        return result.stdout

    def ok(self, *args: str) -> dict:
        envelope = json.loads(self.run(*args))
        assert envelope["kind"] != "error", (self.name, args, envelope)
        return envelope["data"]

    def book(self, fiat: str) -> None:
        self.ok("init")
        self.ok("workspaces", "create", "Main")
        self.ok("profiles", "create", "--workspace", "Main", "--fiat-currency", fiat, "Book")
        self.ok("wallets", "create", *SCOPE, "--label", "W", "--kind", "custom")
        self.ok("wallets", "import-csv", *SCOPE, "--wallet", "W", "--file", str(self.csv))

    def stored(self) -> dict:
        with sqlite3.connect(self.root / "kassiber.sqlite3") as conn:
            return {table: [list(row) for row in conn.execute(sql)] for table, sql in STORED.items()}


def _mask(text: str) -> str:
    ids: dict[str, str] = {}
    text = UUID.sub(lambda m: ids.setdefault(m.group(0), f"<id-{len(ids)}>"), text)
    return INSTANT.sub(lambda m: m.group(0) if m.group(0) in BOOK_DATES else "<time>", text)


def _switch(runtime: Runtime, fiat: str, switch: tuple[str, ...]) -> list[str]:
    """The outputs of a switch and everything a user reads after it."""
    runtime.book(fiat)
    outputs = [runtime.run("profiles", "set", *SCOPE, *switch)]
    outputs.append(runtime.run("journals", "process", *SCOPE))
    outputs.extend(runtime.run(*report, *SCOPE) for report in REPORTS)
    outputs.append(json.dumps(runtime.stored(), sort_keys=True))
    return [_mask(output) for output in outputs]


@pytest.mark.parametrize(
    ("fiat", "switch"),
    [("EUR", ("--fiat-currency", "USD")), ("USD", ("--tax-country", "at"))],
    ids=["eur-book-to-usd", "usd-book-to-at"],
)
def test_a_currency_switch_holds_the_same_rows_on_both_runtimes(tmp_path, fiat, switch):
    python = _switch(Runtime("python", tmp_path), fiat, switch)
    rust = _switch(Runtime("rust", tmp_path), fiat, switch)
    for index, (expected, actual) in enumerate(zip(python, rust, strict=True)):
        assert actual == expected, index
    held = json.loads(python[-1])["journal_quarantines"]
    assert [row[:2] for row in held] == [["buy-1", "price_currency_mismatch"],
                                         ["sell-1", "price_currency_mismatch"]]


def test_a_price_override_in_the_book_currency_releases_the_row_on_both_runtimes(tmp_path):
    outputs = {}
    for name in ("python", "rust"):
        runtime = Runtime(name, tmp_path)
        runtime.book("EUR")
        runtime.ok("profiles", "set", *SCOPE, "--fiat-currency", "USD")
        runtime.ok("journals", "process", *SCOPE)
        held = runtime.ok("journals", "quarantined", *SCOPE)
        buy = next(row["transaction_id"] for row in held if row["occurred_at"].startswith("2024-01"))
        runtime.ok("journals", "quarantine", "resolve", "price-override", *SCOPE,
                   "--transaction", buy, "--fiat-rate", "43000")
        runtime.ok("journals", "process", *SCOPE)
        outputs[name] = [_mask(runtime.run(*report, *SCOPE)) for report in REPORTS]
        outputs[name].append(_mask(json.dumps(runtime.stored(), sort_keys=True)))
    assert outputs["rust"] == outputs["python"]
    # The repriced buy values the holding; the held EUR sale never does.
    sheet = json.loads(outputs["python"][3])["data"]
    assert [(row["quantity"], row["market_value"]) for row in sheet] == [(1.0, 43000.0)]


@pytest.mark.parametrize(
    ("country", "currency", "extra"),
    [
        ("us", "USD", ()),
        ("gb", "GBP", ("--gains-algorithm", "UK_SHARE_MATCHING")),
        ("ch", "CHF", ()),
        ("jp", "JPY", ("--gains-algorithm", "JP_TOTAL_AVERAGE")),
    ],
)
def test_a_rust_only_country_holds_the_eur_prices(tmp_path, country, currency, extra):
    runtime = Runtime("rust", tmp_path)
    runtime.book("EUR")
    update = runtime.ok("profiles", "set", *SCOPE, "--tax-country", country, *extra)
    assert (update["tax_country"], update["fiat_currency"]) == (country, currency)
    assert update["price_currency_holds"] == {
        "reason": "price_currency_mismatch",
        "book_currency": currency,
        "transactions": 2,
        "by_currency": {"EUR": 2},
    }
    assert runtime.ok("journals", "process", *SCOPE)["quarantined"] == 2
    for row in runtime.ok("journals", "quarantined", *SCOPE):
        assert row["reason"] == "price_currency_mismatch"
        assert row["category"] == "missing_price"
        assert {key: row["detail"][key] for key in ("price_currency", "book_currency", "required_for")} == {
            "price_currency": "EUR", "book_currency": currency, "required_for": "book_currency_price"}
        assert row["evidence"]["price_currency"] == "EUR"
        assert row["evidence"]["book_currency"] == currency
    assert runtime.ok("reports", "capital-gains", *SCOPE) == []
    assert runtime.ok("reports", "tax-summary", *SCOPE) == []
    stored = runtime.stored()
    # The stored prices keep their currency; nothing is relabelled.
    assert [row[:4] for row in stored["transactions"]] == [
        ["buy-1", "EUR", 40000.0, "40000"], ["sell-1", "EUR", 60000.0, "60000"]]
    assert stored["journal_entries"] == [[0]]

"""Contract of the frozen tax oracle (``tests/tax_oracle/``, plan 20 phase 4).

The oracle is the phase-3 adapter with its normalizer, Austrian tracker and
pair allocation, copied with the edits ``README.md`` lists and run on RP2.
These checks keep it what it claims to be: the copies match ``FROZEN.json``,
its policy equals the product's, the product's Austrian constants equal RP2's
AT and the native AT (and its Neu cutoff RP2's), RP2 still offers what the
adapter calls and loads without writing a log, and ``projection_range``
accepts what the projection builds and names each shape it cannot build.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from kassiber import tax_policy
from kassiber.core.engines import native
from tests.custody_tax_helpers import finalized_tax_inputs
from tests.tax_ledger_generator import WALLET_REFS, finalized, ledger_history, rail_history, random_inputs
from tests.tax_ledger_generator import profile as ledger_profile
from tests.tax_oracle import adapter, policy, projection_range
from tests.tax_oracle.adapter import OracleTaxEngine

_ROOT = Path(__file__).resolve().parents[2]
_ORACLE = Path(__file__).resolve().parent


def _run_python(script: str, *, cwd: Path) -> subprocess.CompletedProcess:
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(path for path in (str(_ROOT), env.get("PYTHONPATH")) if path)
    return subprocess.run(
        [sys.executable, "-c", script],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )


class FrozenCopyTest(unittest.TestCase):
    def test_copies_match_their_recorded_hashes(self):
        frozen = json.loads((_ORACLE / "FROZEN.json").read_text(encoding="utf-8"))
        self.assertEqual(set(frozen["files"]), {"adapter.py", "austrian.py", "normalize.py", "pair_allocation.py"})
        for name, record in frozen["files"].items():
            with self.subTest(name):
                digest = hashlib.sha256((_ORACLE / name).read_bytes()).hexdigest()
                self.assertEqual(
                    digest,
                    record["sha256"],
                    f"tests/tax_oracle/{name} changed; README.md says when it may and how to record it",
                )

    def test_the_adapter_exposes_only_the_oracle_class(self):
        self.assertEqual(adapter.__all__, ["OracleTaxEngine"])
        self.assertFalse(hasattr(adapter, "GenericRP2TaxEngine"))


class OraclePolicyTest(unittest.TestCase):
    PROFILES = (
        {"tax_country": "at"},
        {"tax_country": " AT ", "fiat_currency": "usd", "tax_long_term_days": 30},
        {"tax_country": "generic", "fiat_currency": "usd", "tax_long_term_days": 30},
        {"fiat_currency": "eur"},
    )

    def test_oracle_policy_equals_the_products_on_both_backends(self):
        for backend in ("rp2", "native"):
            for profile in self.PROFILES:
                with self.subTest(backend=backend, profile=profile), adapter._use_tax_engine_backend(backend):
                    self.assertEqual(policy.build_tax_policy(profile), tax_policy.build_tax_policy(profile))

    def test_austrian_constants_equal_rp2s_at_and_the_native_at(self):
        from rp2.plugin.country import at as rp2_at

        for country in (rp2_at.AT(), native.AT()):
            with self.subTest(type(country).__module__):
                self.assertEqual(country.currency_iso_code.upper(), tax_policy.AUSTRIAN_FIAT_CURRENCY)
                self.assertEqual(country.get_long_term_capital_gain_period(), tax_policy.AUSTRIAN_LONG_TERM_DAYS)
                self.assertEqual(set(country.get_accounting_methods()), set(tax_policy.AUSTRIAN_ACCOUNTING_METHODS))
                self.assertEqual(country.get_default_accounting_method(), tax_policy.AUSTRIAN_DEFAULT_ACCOUNTING_METHOD)
                self.assertEqual(tuple(sorted(country.get_report_generators())), tax_policy.AUSTRIAN_REPORT_GENERATORS)
                self.assertEqual(country.get_default_generation_language(), tax_policy.AUSTRIAN_GENERATION_LANGUAGE)

    def test_neu_cutoff_equals_rp2s(self):
        # The Alt/Neu cutoff the product's Vienna helpers and reports use is
        # the one RP2's AT plugin classifies by.
        from rp2.plugin.country.at import AT_NEU_CUTOFF as RP2_AT_NEU_CUTOFF

        from kassiber.core.austrian import AT_NEU_CUTOFF

        self.assertEqual(AT_NEU_CUTOFF, RP2_AT_NEU_CUTOFF)

    def test_iso_4217_codes_match_pycountry(self):
        # RP2 validates the fiat currency against pycountry; production's
        # request builder and the legacy-native leg check the same list.
        import pycountry

        from kassiber.core.engines import ledger_request

        self.assertEqual(ledger_request.ISO_4217, frozenset(currency.alpha_3 for currency in pycountry.currencies))
        self.assertIs(native._ISO_4217, ledger_request.ISO_4217)


def _austrian_swap_inputs():
    profile = {
        "id": "profile-at",
        "workspace_id": "workspace-main",
        "label": "FixtureAustrianSwap",
        "fiat_currency": "EUR",
        "tax_country": "at",
        "tax_long_term_days": 9223372036854775807,
        "gains_algorithm": "MOVING_AVERAGE_AT",
    }
    wallets = {
        wallet_id: {
            "id": wallet_id,
            "label": label,
            "wallet_account_id": "account-treasury",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        for wallet_id, label in (("wallet-a", "Vienna"), ("wallet-b", "Liquid"))
    }

    def row(row_id, wallet_id, occurred_at, direction, asset, amount, fiat_rate, fiat_value, description, external_id):
        return {
            "id": row_id,
            "wallet_id": wallet_id,
            "wallet_label": wallets[wallet_id]["label"],
            "wallet_account_id": "account-treasury",
            "account_code": "treasury",
            "account_label": "Treasury",
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount": amount,
            "fee": 0,
            "fiat_rate": fiat_rate,
            "fiat_value": fiat_value,
            "kind": "deposit" if direction == "inbound" else "withdrawal",
            "description": description,
            "note": None,
            "external_id": external_id,
            "created_at": occurred_at,
        }

    rows = [
        row(
            "neu-buy-1", "wallet-a", "2024-06-01T10:00:00Z", "inbound", "BTC", 100_000_000_000, 30000, 30000,
            "Vienna Neu buy", "neu-buy-1",
        ),
        row(
            "swap-out-1", "wallet-a", "2025-03-01T09:00:00Z", "outbound", "BTC", 50_000_000_000, 50000, 25000,
            "Peg-out BTC->LBTC", "swap-1-out",
        ),
        row(
            "swap-in-1", "wallet-b", "2025-03-01T09:00:00Z", "inbound", "LBTC", 50_000_000_000, 50000, 25000,
            "Peg-in BTC->LBTC", "swap-1-in",
        ),
    ]
    pairs = [
        {
            "id": "mp-at-swap-1",
            "out_transaction_id": "swap-out-1",
            "in_transaction_id": "swap-in-1",
            "policy": "carrying-value",
            "kind": "swap",
        }
    ]
    return profile, finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=wallets, manual_pair_records=pairs)


class Rp2InterfaceTest(unittest.TestCase):
    def test_rp2s_austrian_country_exposes_validate_input_data(self):
        """The adapter calls ``configuration.country.validate_input_data`` to
        check cross-asset swap links (``_validate_prepared_rp2_inputs``). The
        dependency pin keeps the rp2 commit in sync across files, but only
        this check guarantees the pinned fork still exposes the method; a
        commit that dropped it would pass drift and then fail every Austrian
        oracle run with the ``unsupported`` AppError.
        """

        from rp2.plugin.country.at import AT

        self.assertTrue(
            callable(getattr(AT, "validate_input_data", None)),
            "Pinned rp2.plugin.country.at.AT no longer exposes validate_input_data; "
            "bump the rp2 pin in pyproject.toml / uv.lock / THIRD_PARTY_LICENSES.md "
            "to a fork commit that restores the hook.",
        )

    def test_austrian_cross_asset_swap_uses_the_multi_asset_hook(self):
        profile, inputs = _austrian_swap_inputs()
        # The Austrian country of the oracle's active backend.
        AT = adapter._load_at_country_module().AT
        calls: list[set[str]] = []
        original = AT.compute_tax_for_assets

        def spy(self, configuration, accounting_engine, asset_to_input_data):
            calls.append(set(asset_to_input_data))
            return original(self, configuration, accounting_engine, asset_to_input_data)

        AT.compute_tax_for_assets = spy  # type: ignore[assignment]
        try:
            OracleTaxEngine(profile).build_ledger_state(inputs)
        finally:
            AT.compute_tax_for_assets = original  # type: ignore[assignment]

        self.assertEqual(calls, [{"BTC", "LBTC"}])


class Rp2ImportTest(unittest.TestCase):
    """Older ``rp2.logger`` releases bound a ``logging.FileHandler`` to
    ``./log/rp2_<timestamp>.log`` at import time, which crashed with EACCES
    under a read-only cwd. ``_get_rp2_modules`` primes ``rp2.logger`` under a
    writable scratch cwd first and removes the handler; a fresh interpreter
    pins both.
    """

    def test_get_rp2_modules_succeeds_under_read_only_cwd(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="kassiber-ro-cwd-"))
        try:
            os.chmod(scratch, stat.S_IRUSR | stat.S_IXUSR)
            script = textwrap.dedent(
                """
                from tests.tax_oracle.adapter import _get_rp2_modules, _use_tax_engine_backend

                with _use_tax_engine_backend("rp2"):
                    modules = _get_rp2_modules()
                assert "InTransaction" in modules, modules.keys()
                assert "Configuration" in modules, modules.keys()
                print("ok")
                """
            ).strip()
            result = _run_python(script, cwd=scratch)
            self.assertEqual(
                result.returncode,
                0,
                msg=(
                    "rp2 import under a read-only cwd should succeed; "
                    f"stdout={result.stdout!r} stderr={result.stderr!r}"
                ),
            )
            self.assertIn("ok", result.stdout)
        finally:
            os.chmod(scratch, stat.S_IRWXU)
            shutil.rmtree(scratch, ignore_errors=True)

    def test_get_rp2_modules_leaves_no_file_logger_or_log_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-rp2-log-test-") as tmp:
            script = textwrap.dedent(
                f"""
                import logging
                import pathlib
                import tempfile

                tempfile.tempdir = {str(tmp)!r}

                from tests.tax_oracle.adapter import _get_rp2_modules, _use_tax_engine_backend

                with _use_tax_engine_backend("rp2"):
                    modules = _get_rp2_modules()
                assert "InTransaction" in modules, modules.keys()
                root = pathlib.Path(tempfile.gettempdir())
                log_files = list(root.rglob("rp2_*.log"))
                scratch_dirs = list(root.glob("kassiber-rp2-logs-*"))
                handlers = logging.getLogger("rp2").handlers
                assert not log_files, [str(path) for path in log_files]
                assert not scratch_dirs, [str(path) for path in scratch_dirs]
                assert not any(isinstance(handler, logging.FileHandler) for handler in handlers), handlers
                assert any(isinstance(handler, logging.NullHandler) for handler in handlers), handlers
                print("ok")
                """
            ).strip()
            result = _run_python(script, cwd=_ROOT)
            self.assertEqual(
                result.returncode,
                0,
                msg=(
                    "rp2 import should leave no disk logger; "
                    f"stdout={result.stdout!r} stderr={result.stderr!r}"
                ),
            )
            self.assertIn("ok", result.stdout)


def _projection_books(country):
    """``(label, profile, inputs)`` of projection-built books for ``country``."""
    profile = ledger_profile(country, "fifo")
    books = [(name, profile, finalized(profile, *history())) for name, history in (("ledger", ledger_history), ("rail", rail_history))]
    for seed in range(24):
        book = random_inputs(seed, country)
        books.append((f"seed {seed}", book.profile, book.inputs))
    return books


def _changed(inputs, change):
    """``inputs`` with ``change(rows, pairs)`` applied to copies of its rows and pairs."""
    projection = inputs.finalized_tax_projection
    rows = [dict(row) for row in projection.rows]
    by_id = {row["id"]: row for row in rows}
    pairs = [{**pair, "out": by_id[pair["out"]["id"]], "in": by_id[pair["in"]["id"]]} for pair in projection.intra_pairs]
    change(rows, pairs)
    changed = dataclasses.replace(projection, rows=tuple(rows), intra_pairs=tuple(pairs))
    return dataclasses.replace(inputs, finalized_tax_projection=changed)


def _standalone(rows, pairs, direction):
    paired = {pair[side]["id"] for pair in pairs for side in ("out", "in")}
    return next(row for row in rows if row["direction"] == direction and row["id"] not in paired)


def _set(field, value, *, side="inbound", **more):
    """Set ``field`` (and ``more``) on the first unpaired row of ``side``."""

    def change(rows, pairs):
        _standalone(rows, pairs, side).update({field: value, **more})

    return change


def _set_privacy(rows, pairs):
    pairs[0]["out"]["privacy_boundary"] = "CoinJoin"


def _set_privacy_in(rows, pairs):
    pairs[0]["in"]["privacy_boundary"] = "sweep"


def _share_leg(rows, pairs):
    pairs.append(dict(pairs[0]))


def _unequal_move(rows, pairs):
    pairs[0]["in"] = {**pairs[0]["in"], "amount": pairs[0]["in"]["amount"] - 1}


class ProjectionRangeTest(unittest.TestCase):
    def test_projection_built_books_are_in_range(self):
        for country in ("generic", "at"):
            for label, profile, inputs in _projection_books(country):
                with self.subTest(country=country, book=label):
                    self.assertEqual(projection_range.violations(profile, inputs), ())

    def test_each_shape_the_projection_cannot_build_is_named(self):
        profile = ledger_profile("generic", "fifo")
        inputs = finalized(profile, *ledger_history())
        wallets = dict(WALLET_REFS, A={**WALLET_REFS["A"], "id": "other"})
        cases = {
            "non_global_pool_scope": (dict(profile, cost_basis_pool_scope="wallet"), inputs),
            "wallet_ref_id_mismatch": (profile, dataclasses.replace(inputs, wallet_refs_by_id=wallets)),
            "non_synthetic_row_id": (profile, _changed(inputs, _set("id", "buy-1"))),
            "missing_anchor": (profile, _changed(inputs, _set("journal_transaction_id", None))),
            "unknown_wallet": (profile, _changed(inputs, _set("wallet_id", "Z"))),
            "unknown_direction": (profile, _changed(inputs, _set("direction", "sideways"))),
            "non_integer_msat": (profile, _changed(inputs, _set("amount", 1.5))),
            "negative_fee": (profile, _changed(inputs, _set("fee", -1))),
            "non_positive_amount": (profile, _changed(inputs, _set("amount", 0))),
            "amount_includes_fee": (profile, _changed(inputs, _set("amount_includes_fee", True))),
            "custody_component_fields": (profile, _changed(inputs, _set("custody_component_id", None))),
            "corrupt_decimal_text": (profile, _changed(inputs, _set("fiat_rate_exact", "4O000"))),
            "group_block_rows": (profile, _changed(inputs, lambda rows, pairs: pairs[0].update(group_block_rows=(rows[0],)))),
            "unreviewed_privacy_pair": (profile, _changed(inputs, _set_privacy)),
            "shared_pair_leg": (profile, _changed(inputs, _share_leg)),
            "unequal_move_amounts": (profile, _changed(inputs, _unequal_move)),
        }
        self.assertEqual(tuple(cases), projection_range.INVARIANTS)
        for name, (case_profile, case_inputs) in cases.items():
            with self.subTest(name):
                self.assertEqual(projection_range.violations(case_profile, case_inputs), (name,))

    def test_each_branch_of_a_rule_is_named(self):
        profile = ledger_profile("generic", "fifo")
        inputs = finalized(profile, *ledger_history())
        cases = (
            ("outbound amount and fee zero", _set("amount", 0, side="outbound", fee=0), ("non_positive_amount",)),
            ("outbound fee alone", _set("amount", 0, side="outbound"), ()),
            ("non-integer fee", _set("fee", 1.5), ("non_integer_msat",)),
            ("corrupt exact value", _set("fiat_value_exact", "6l000"), ("corrupt_decimal_text",)),
            ("corrupt value fallback", _set("fiat_value", "6l000", fiat_value_exact=None), ("corrupt_decimal_text",)),
            ("privacy on the in leg", _set_privacy_in, ("unreviewed_privacy_pair",)),
        )
        for label, change, expected in cases:
            with self.subTest(label):
                self.assertEqual(projection_range.violations(profile, _changed(inputs, change)), expected)

    def test_a_reviewed_privacy_pair_is_in_range(self):
        profile = ledger_profile("generic", "fifo")
        inputs = finalized(profile, *ledger_history())

        def reviewed(rows, pairs):
            _set_privacy(rows, pairs)
            pairs[0]["kind"] = projection_range.REVIEWED_PRIVACY_PAIR_KIND

        self.assertEqual(projection_range.violations(profile, _changed(inputs, reviewed)), ())


if __name__ == "__main__":
    unittest.main()

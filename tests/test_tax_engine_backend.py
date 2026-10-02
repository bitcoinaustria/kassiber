"""The native tax-engine backend: isolation, binding, and RP2-shaped interface.

``kassiber/core/engines/native.py`` stands in for RP2 beneath the adapter.
These tests pin what parity runs cannot show: that it runs with RP2 absent,
writes nothing, keeps RP2's values, checks and messages at RP2's phase, and
reaches the binding through its versioned JSON contract.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from decimal import ROUND_HALF_EVEN, Decimal
from pathlib import Path

import kassiber_tax

from kassiber.core.engines import native, native_request
from kassiber.core.engines import rp2 as adapter
from kassiber.tax_policy import build_tax_policy
from tests import tax_engine_compare as compare

_ROOT = Path(__file__).resolve().parents[1]

# RP2 and the packages only it brings in.
_BLOCK_RP2 = textwrap.dedent(
    """
    import importlib.abc
    import sys

    class _BlockRP2(importlib.abc.MetaPathFinder):
        def find_spec(self, name, path=None, target=None):
            if name.split(".")[0] in {"rp2", "prezzemolo", "pycountry"}:
                raise ModuleNotFoundError(f"No module named {name!r} (blocked)", name=name)
            return None

    sys.meta_path.insert(0, _BlockRP2())
    """
)


def _run_python(script: str, *, cwd: Path) -> subprocess.CompletedProcess:
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(path for path in (str(_ROOT), env.get("PYTHONPATH")) if path)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    return subprocess.run(
        [sys.executable, "-c", script],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=300,
    )


class NativeBackendIsolationTest(unittest.TestCase):
    def test_runs_journals_without_rp2_installed(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-native-norp2-") as scratch:
            script = _BLOCK_RP2 + textwrap.dedent(
                f"""
                import tempfile
                tempfile.tempdir = {scratch!r}

                from kassiber.core.engines import GenericRP2TaxEngine
                from kassiber.core.engines import rp2 as adapter
                from kassiber.errors import AppError
                from kassiber.tax_policy import build_tax_policy
                from tests.custody_tax_helpers import finalized_tax_inputs
                from tests.test_tax_engine_parity import WALLET_REFS, _adapter_profile, _ledger_history, _rail_history

                with adapter._use_tax_engine_backend("native"):
                    # An Austrian swap needs moving_average_at; under fifo it fails closed.
                    books = (_ledger_history(), _rail_history())
                    runs = (("generic", "FIFO", books), ("generic", "hifo", books), ("at", "fifo", books[:1]), ("at", "moving_average_at", books))
                    for country, method, histories in runs:
                        profile = _adapter_profile(country, method)
                        for rows, pairs in histories:
                            result = GenericRP2TaxEngine(profile).build_ledger_state(
                                finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
                            )
                            realized = [entry for entry in result.entries if "calculation" in entry]
                            assert realized, (country, method)
                            assert {{entry["calculation"]["engine"] for entry in realized}} == {{"kassiber_tax"}}
                    assert build_tax_policy({{"tax_country": "at"}}).default_accounting_method == "moving_average_at"

                # The RP2 backend still fails closed with its own message.
                try:
                    with adapter._use_tax_engine_backend("rp2"):
                        adapter._get_rp2_modules()
                except AppError as exc:
                    assert str(exc).startswith("RP2 integration requires the 'rp2' package."), str(exc)
                else:
                    raise AssertionError("rp2 was importable")
                loaded = sorted(name for name in sys.modules if name.split(".")[0] in {{"rp2", "prezzemolo", "pycountry"}})
                assert not loaded, loaded
                print("ok")
                """
            )
            result = _run_python(script, cwd=Path(scratch))
        self.assertEqual(result.returncode, 0, msg=f"stdout={result.stdout!r} stderr={result.stderr[-4000:]!r}")
        self.assertIn("ok", result.stdout)

    def test_writes_no_files_and_opens_no_sockets(self):
        # An audit hook records every write-mode open, file-system change and
        # socket call during one journal run per backend. The RP2 run proves
        # the hook sees its temporary INI file.
        with tempfile.TemporaryDirectory(prefix="kassiber-native-io-") as scratch:
            temp_root = Path(scratch) / "tmp"
            work = Path(scratch) / "cwd"
            temp_root.mkdir()
            work.mkdir()
            script = textwrap.dedent(
                f"""
                import json
                import os
                import sys
                import tempfile
                tempfile.tempdir = {str(temp_root)!r}

                from kassiber.core.engines import GenericRP2TaxEngine
                from kassiber.core.engines import rp2 as adapter
                from tests.custody_tax_helpers import finalized_tax_inputs
                from tests.test_tax_engine_parity import WALLET_REFS, _adapter_profile, _ledger_history

                profile = _adapter_profile("generic", "FIFO")
                rows, pairs = _ledger_history()
                inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
                WRITE_FLAGS = os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_APPEND | os.O_TRUNC
                EVENTS = {{
                    "os.mkdir", "os.remove", "os.rename", "os.rmdir", "os.truncate", "os.chdir",
                    "tempfile.mkstemp", "tempfile.mkdtemp", "socket.connect", "socket.getaddrinfo", "socket.bind",
                }}
                recording = []
                state = {{"active": False}}

                def hook(event, args):
                    if not state["active"]:
                        return
                    if event == "open" and isinstance(args[2], int) and args[2] & WRITE_FLAGS:
                        recording.append([event, str(args[0])])
                    elif event in EVENTS:
                        recording.append([event, str(args[0]) if args else ""])

                sys.addaudithook(hook)
                engine = GenericRP2TaxEngine(profile)
                for backend in ("native", "rp2"):  # warm up lazy imports
                    with adapter._use_tax_engine_backend(backend):
                        engine.build_ledger_state(inputs)
                events = {{}}
                for backend in ("native", "rp2"):
                    recording.clear()
                    state["active"] = True
                    with adapter._use_tax_engine_backend(backend):
                        engine.build_ledger_state(inputs)
                    state["active"] = False
                    events[backend] = list(recording)
                print(json.dumps(events))
                """
            )
            result = _run_python(script, cwd=work)
            self.assertEqual(result.returncode, 0, msg=f"stderr={result.stderr[-4000:]!r}")
            events = json.loads(result.stdout.strip().splitlines()[-1])
            self.assertEqual(events["native"], [])
            self.assertTrue(
                any(event == "open" and path.endswith(".ini") for event, path in events["rp2"]),
                events["rp2"],
            )
            self.assertEqual(sorted(path.name for path in temp_root.iterdir()), [])
            self.assertEqual(sorted(path.name for path in work.iterdir()), [])

    def test_new_threads_compute_at_32_digits_without_rp2(self):
        script = _BLOCK_RP2 + textwrap.dedent(
            """
            import decimal
            import json
            import threading

            import kassiber  # noqa: F401 - the package sets the policy

            seen = {}

            def probe():
                context = decimal.getcontext()
                seen.update(
                    prec=context.prec,
                    rounding=context.rounding,
                    float_trap=bool(context.traps[decimal.FloatOperation]),
                    third=str(decimal.Decimal(1) / decimal.Decimal(3)),
                )

            worker = threading.Thread(target=probe)
            worker.start()
            worker.join()
            main = decimal.getcontext()
            print(json.dumps({
                "thread": seen,
                "main": [main.prec, main.rounding],
                "rp2": any(name.split(".")[0] == "rp2" for name in sys.modules),
            }))
            """
        )
        with tempfile.TemporaryDirectory(prefix="kassiber-decimal-") as scratch:
            result = _run_python(script, cwd=Path(scratch))
        self.assertEqual(result.returncode, 0, msg=f"stderr={result.stderr[-4000:]!r}")
        report = json.loads(result.stdout.strip().splitlines()[-1])
        self.assertFalse(report["rp2"])
        self.assertEqual(report["main"], [32, ROUND_HALF_EVEN])
        self.assertEqual(report["thread"]["prec"], 32)
        self.assertEqual(report["thread"]["rounding"], ROUND_HALF_EVEN)
        self.assertEqual(report["thread"]["third"], "0." + "3" * 32)
        # Traps are untouched: only RP2's import ever trapped FloatOperation.
        self.assertFalse(report["thread"]["float_trap"])


class BindingRoundTripTest(unittest.TestCase):
    ENTRY = dict(
        timestamp="2024-01-01T00:00:00Z",
        asset="BTC",
        exchange="Cold",
        holder="Default",
        transaction_type="BUY",
        spot_price=Decimal("42000.50"),
        crypto_in=Decimal("1E-11"),
        fiat_in_no_fee=Decimal("4.2000500E-7"),
        fiat_in_with_fee=Decimal("4.2000500E-7"),
        fiat_fee=Decimal("0"),
        row=1,
        unique_id="tx-1",
        notes="Exchange buy",
    )

    def test_engine_version_is_reported(self):
        self.assertRegex(kassiber_tax.engine_version(), r"^\d+\.\d+\.\d+")

    def test_check_entry_round_trips_exact_decimals(self):
        entry = native_request.in_entry(**self.ENTRY)
        request = native_request.check_entry_request(entry, assets=["BTC"], exchanges=["Cold"], holders=["Default"])
        response = native_request.loads(kassiber_tax.check_entry(native_request.dumps(request)))
        self.assertTrue(response["ok"])
        derived = response["entry"]
        self.assertEqual(native_request.decode_decimal(derived["crypto_in"]).as_tuple(), Decimal("1E-11").as_tuple())
        self.assertEqual(
            native_request.decode_decimal(derived["fiat_in_with_fee"]).as_tuple(), Decimal("4.2000500E-7").as_tuple()
        )
        self.assertEqual(derived["transaction_type"], "buy")
        self.assertEqual(derived["unique_id"], "tx-1")
        self.assertFalse(derived["is_taxable"])

    def test_compute_round_trip(self):
        buy = native_request.in_entry(**self.ENTRY)
        sell = native_request.out_entry(
            timestamp="2024-02-01T00:00:00Z", asset="BTC", exchange="Cold", holder="Default",
            transaction_type="SELL", spot_price=Decimal("50000"), crypto_out_no_fee=Decimal("5E-12"),
            crypto_fee=Decimal("0"), fiat_out_no_fee=Decimal("2.5E-7"), fiat_fee=Decimal("0"), row=2,
        )
        request = native_request.compute_request(
            method="fifo", assets={"BTC": [buy, sell]}, country=native_request.country_spec("generic", 365)
        )
        response = native_request.loads(kassiber_tax.compute(native_request.dumps(request)))
        self.assertTrue(response["ok"])
        [asset] = response["assets"]
        [gain_loss] = asset["gain_losses"]
        self.assertEqual(gain_loss["event"], {"kind": "out", "row": 2})
        self.assertEqual(gain_loss["lot"], 1)
        self.assertEqual(native_request.decode_decimal(gain_loss["crypto_amount"]), Decimal("5E-12"))

    def test_malformed_requests_fail_as_request_errors(self):
        for text in ("not json", json.dumps({"schema_version": 99, "operation": "compute"})):
            with self.subTest(text=text):
                response = json.loads(kassiber_tax.compute(text))
                self.assertFalse(response["ok"])
                self.assertEqual(response["error"]["class"], "RequestError")
        with self.assertRaises(native.EngineContractError) as raised:
            native._call("compute", {"schema_version": 1, "operation": "compute"})
        self.assertEqual(raised.exception.error_class, "RequestError")

    def test_rust_panic_surfaces_as_runtime_error(self):
        # The binding converts a panic into RuntimeError, but it exposes no
        # entry point that panics on demand.
        self.skipTest("kassiber_tax exposes no way to trigger a panic")


class EngineDecimalTest(unittest.TestCase):
    def test_matches_rp2_decimal_operations(self):
        from rp2.rp2_decimal import RP2Decimal

        values = ["0", "-0", "1", "1.00000000000005", "1.00000000000006", "0.1", "3", "100.50", "1E-11", "-2.5E+3", "9999999999999999999"]
        for left in values:
            for right in values:
                with self.subTest(left=left, right=right):
                    expected = self._operations(RP2Decimal(left), RP2Decimal(right))
                    actual = self._operations(native.EngineDecimal(left), native.EngineDecimal(right))
                    self.assertEqual(expected, actual)

    @staticmethod
    def _operations(left, right):
        results = {}
        for name, operation in (
            ("eq", lambda: left == right), ("ne", lambda: left != right), ("lt", lambda: left < right),
            ("le", lambda: left <= right), ("gt", lambda: left > right), ("ge", lambda: left >= right),
            ("add", lambda: left + right), ("sub", lambda: left - right), ("mul", lambda: left * right),
            ("div", lambda: left / right), ("neg", lambda: -left), ("plain_eq", lambda: Decimal(str(left)) == right),
        ):
            try:
                value = operation()
            except Exception as exc:  # noqa: BLE001 - errors are part of the semantics
                results[name] = ["error", type(exc).__name__]
                continue
            results[name] = [str(value), isinstance(value, (native.EngineDecimal,)) or type(value).__name__ == "RP2Decimal"]
        return results

    def test_rejects_non_decimal_operands_and_hashing(self):
        value = native.EngineDecimal("1")
        for operation in (lambda: value == 1, lambda: value < 1.5, lambda: value + 1, lambda: 2 * value):
            with self.assertRaises(native.EngineTypeError) as raised:
                operation()
            self.assertTrue(str(raised.exception).startswith("Operand has non-Decimal value"))
        with self.assertRaises(TypeError):
            hash(value)


PROFILE = {
    "id": "p",
    "workspace_id": "w",
    "label": "Default",
    "tax_country": "generic",
    "fiat_currency": "EUR",
    "gains_algorithm": "fifo",
    "tax_long_term_days": 365,
}


def _outcome(backend, build, *, profile=PROFILE, wallets=("Cold", "Hot"), assets=("BTC",)):
    """Run ``build(modules, configuration)`` on ``backend``; return its result or error signature."""

    def call():
        modules = adapter._get_rp2_modules()
        with adapter._rp2_configuration(profile, list(wallets), list(assets)) as configuration:
            return build(modules, configuration)

    result, error = compare.run_backend(backend, call)
    return ("error", compare.error_signature(error)) if error is not None else ("ok", result)


def _in(modules, configuration, **overrides):
    decimal = modules["RP2Decimal"]
    arguments = dict(
        configuration=configuration, timestamp="2024-01-01T00:00:00Z", asset="BTC", exchange="Cold",
        holder="Default", transaction_type="BUY", spot_price=decimal("100"), crypto_in=decimal("1"),
        fiat_in_no_fee=decimal("100"), fiat_in_with_fee=decimal("100"), fiat_fee=decimal("0"), row=1,
        unique_id="u", notes="",
    )
    arguments.update(overrides)
    return modules["InTransaction"](**arguments)


class InterfaceParityTest(unittest.TestCase):
    """Checks raised at RP2's phase, with RP2's messages."""

    def assertSameOutcome(self, build, **kwargs):
        expected = _outcome("rp2", build, **kwargs)
        actual = _outcome("native", build, **kwargs)
        self.assertEqual(expected, actual)
        return expected

    def test_constructor_errors(self):
        cases = {
            "zero crypto_in": lambda m, c: _in(m, c, crypto_in=m["RP2Decimal"]("0")),
            "unknown exchange": lambda m, c: _in(m, c, exchange="Nowhere"),
            "padded holder": lambda m, c: _in(m, c, holder=" Default "),
            "naive timestamp": lambda m, c: _in(m, c, timestamp="2024-01-01"),
            "bad timestamp": lambda m, c: _in(m, c, timestamp="not a date"),
            "non-string timestamp": lambda m, c: _in(m, c, timestamp=20240101),
            "sell type on an in": lambda m, c: _in(m, c, transaction_type="SELL"),
            "unknown type": lambda m, c: _in(m, c, transaction_type=" BUY"),
            "spot below 5E-14": lambda m, c: _in(m, c, spot_price=m["RP2Decimal"]("4E-14")),
            "huge fiat": lambda m, c: _in(m, c, fiat_in_no_fee=m["RP2Decimal"]("1E+19")),
            "plain decimal": lambda m, c: _in(m, c, spot_price=Decimal("100")),
            "unknown asset": lambda m, c: _in(m, c, asset="ETH"),
            "fee without spot": lambda m, c: m["IntraTransaction"](
                configuration=c, timestamp="2024-01-01T00:00:00Z", asset="BTC", from_exchange="Cold",
                from_holder="Default", to_exchange="Hot", to_holder="Default", spot_price=m["RP2Decimal"]("0"),
                crypto_sent=m["RP2Decimal"]("0.5"), crypto_received=m["RP2Decimal"]("0.4"), row=2, unique_id="tx3",
            ),
            "received above sent": lambda m, c: m["IntraTransaction"](
                configuration=c, timestamp="2024-01-01T00:00:00Z", asset="BTC", from_exchange="Cold",
                from_holder="Default", to_exchange="Hot", to_holder="Default", spot_price=m["RP2Decimal"]("10"),
                crypto_sent=m["RP2Decimal"]("0.4"), crypto_received=m["RP2Decimal"]("0.5"), row=2,
            ),
            "fee row with amount": lambda m, c: m["OutTransaction"](
                configuration=c, timestamp="2024-01-01T00:00:00Z", asset="BTC", exchange="Cold", holder="Default",
                transaction_type="FEE", spot_price=m["RP2Decimal"]("10"), crypto_out_no_fee=m["RP2Decimal"]("1"),
                crypto_fee=m["RP2Decimal"]("0.1"), fiat_fee=m["RP2Decimal"]("1"), row=3,
            ),
        }
        for name, build in cases.items():
            with self.subTest(name):
                outcome = self.assertSameOutcome(build)
                self.assertEqual(outcome[0], "error", outcome)

    def test_constructed_values_and_text(self):
        def build(modules, configuration):
            decimal = modules["RP2Decimal"]
            transactions = [
                _in(modules, configuration, notes="at_regime=neu Buy", fiat_in_no_fee=decimal("100.00"), fiat_in_with_fee=decimal("100.00")),
                modules["OutTransaction"](
                    configuration=configuration, timestamp="2024-02-01T00:00:00+01:00", asset="BTC", exchange="Cold",
                    holder="Default", transaction_type="FEE", spot_price=decimal("50000"), crypto_out_no_fee=decimal("0"),
                    crypto_fee=decimal("0.00002"), fiat_out_no_fee=None, fiat_fee=decimal("1.00000"), row=2, unique_id="f",
                ),
                modules["IntraTransaction"](
                    configuration=configuration, timestamp="2024-03-01T00:00:00.250000Z", asset="BTC", from_exchange="Cold",
                    from_holder="Default", to_exchange="Hot", to_holder="Default", spot_price=decimal("0"),
                    crypto_sent=decimal("0.05"), crypto_received=decimal("0.05"), row=3, unique_id="m",
                ),
            ]
            views = []
            for transaction in transactions:
                view = {
                    name: str(getattr(transaction, name))
                    for name in (
                        "internal_id", "unique_id", "notes", "asset", "spot_price", "crypto_fee", "fiat_fee",
                        "crypto_balance_change", "crypto_taxable_amount", "fiat_taxable_amount",
                    )
                }
                view["timestamp"] = transaction.timestamp.isoformat()
                view["type"] = [transaction.transaction_type.value, transaction.transaction_type.name, transaction.transaction_type.is_earn_type()]
                view["taxable"] = [transaction.is_taxable(), transaction.is_earning()]
                view["text"] = str(transaction)
                views.append(view)
            return views

        outcome = self.assertSameOutcome(build)
        self.assertEqual(outcome[0], "ok")

    def test_set_checks(self):
        def duplicate_row(modules, configuration):
            in_set = modules["TransactionSet"](configuration, "IN", "BTC")
            in_set.add_entry(_in(modules, configuration))
            in_set.add_entry(_in(modules, configuration, unique_id="other"))

        def wrong_kind(modules, configuration):
            modules["TransactionSet"](configuration, "OUT", "BTC").add_entry(_in(modules, configuration))

        def wrong_asset(modules, configuration):
            modules["TransactionSet"](configuration, "IN", "LBTC").add_entry(_in(modules, configuration))

        def bogus_set(modules, configuration):
            modules["TransactionSet"](configuration, "BOGUS", "BTC")

        def unknown_set_asset(modules, configuration):
            modules["TransactionSet"](configuration, "IN", "ETH")

        def duplicate_unique_id(modules, configuration):
            in_set = modules["TransactionSet"](configuration, "in", "BTC")
            in_set.add_entry(_in(modules, configuration))
            in_set.add_entry(_in(modules, configuration, row=2))
            return in_set.count

        for build in (duplicate_row, wrong_kind, wrong_asset, bogus_set, unknown_set_asset):
            with self.subTest(build.__name__):
                outcome = self.assertSameOutcome(build, assets=("BTC", "LBTC"))
                self.assertEqual(outcome[0], "error")
        self.assertEqual(self.assertSameOutcome(duplicate_unique_id), ("ok", 2))

    def test_empty_in_set_is_rejected(self):
        def build(modules, configuration):
            sets = [modules["TransactionSet"](configuration, kind, "BTC") for kind in ("IN", "OUT", "INTRA")]
            modules["InputData"](
                asset="BTC",
                unfiltered_in_transaction_set=sets[0],
                unfiltered_out_transaction_set=sets[1],
                unfiltered_intra_transaction_set=sets[2],
            )

        for backend, name in (("rp2", "RP2ValueError"), ("native", "EngineValueError")):
            with self.subTest(backend):
                with adapter._use_tax_engine_backend(backend):
                    modules = adapter._get_rp2_modules()
                    with adapter._rp2_configuration(PROFILE, ["Cold"], ["BTC"]) as configuration:
                        with self.assertRaises(Exception) as raised:
                            build(modules, configuration)
                # RP2's message embeds its INI path; the prefix is the contract.
                self.assertEqual(type(raised.exception).__name__, name)
                self.assertTrue(str(raised.exception).startswith("IN transaction set is empty: TransactionSet:"))

    def test_configuration_failures(self):
        def nothing(modules, configuration):
            return sorted(modules)

        cases = [
            ("missing currency", dict(PROFILE, fiat_currency=None), ("Cold",)),
            ("unknown currency", dict(PROFILE, fiat_currency="XYZ"), ("Cold",)),
            ("percent label", PROFILE, ("50% cold",)),
            ("interpolated label", PROFILE, ("a%(x)s",)),
            ("delimited label", PROFILE, ("Cold, Savings",)),
        ]
        for name, profile, wallets in cases:
            with self.subTest(name):
                outcome = self.assertSameOutcome(nothing, profile=profile, wallets=wallets)
                self.assertEqual(outcome[0], "error")

    def test_duplicate_labels_after_strip_fail_on_both(self):
        # RP2's message carries its temporary INI path; the native one lacks it.
        for backend in ("rp2", "native"):
            with self.subTest(backend), adapter._use_tax_engine_backend(backend):
                with self.assertRaises(Exception) as raised:
                    with adapter._rp2_configuration(PROFILE, ["W1", " W1 "], ["BTC"]):
                        pass
                self.assertTrue(
                    str(raised.exception).endswith(
                        "field 'exchanges' in section 'general' contains duplicate elements: W1"
                    ),
                    str(raised.exception),
                )

    def test_modules_mirror_the_rp2_names(self):
        with adapter._use_tax_engine_backend("rp2"):
            rp2_names = set(adapter._get_rp2_modules())
        with adapter._use_tax_engine_backend("native"):
            self.assertEqual(set(adapter._get_rp2_modules()), rp2_names - {"BalanceSet"})

    def test_backend_selection_is_scoped_and_validated(self):
        # The native engine is the default unless this session selected RP2.
        session = os.environ.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower()
        self.assertEqual(adapter._tax_engine_backend(), "rp2" if session == "rp2" else "native")
        before = adapter._tax_engine_backend()
        with adapter._use_tax_engine_backend("native"):
            self.assertEqual(adapter._tax_engine_backend(), "native")
        self.assertEqual(adapter._tax_engine_backend(), before)
        with self.assertRaises(ValueError):
            with adapter._use_tax_engine_backend("other"):
                pass


class CountryPolicyTest(unittest.TestCase):
    def test_austrian_country_matches_rp2_policy(self):
        from rp2.plugin.country import at as rp2_at

        expected, actual = rp2_at.AT(), native.AT()
        for name in (
            "country_iso_code", "currency_iso_code",
        ):
            self.assertEqual(getattr(expected, name), getattr(actual, name))
        for name in (
            "get_long_term_capital_gain_period", "get_default_accounting_method", "get_accounting_methods",
            "get_report_generators", "get_default_generation_language", "get_default_application_method",
            "get_application_methods",
        ):
            with self.subTest(name):
                self.assertEqual(getattr(expected, name)(), getattr(actual, name)())
        self.assertEqual(actual.get_long_term_capital_gain_period(), sys.maxsize)
        self.assertEqual({item.value for item in rp2_at.AtDisposalCategory}, {item.value for item in native.AtDisposalCategory})

    def test_tax_policy_is_the_same_on_both_backends(self):
        for profile in ({"tax_country": "at"}, {"tax_country": "generic", "fiat_currency": "usd", "tax_long_term_days": 30}):
            with self.subTest(profile=profile):
                expected = build_tax_policy(profile)
                with adapter._use_tax_engine_backend("native"):
                    self.assertEqual(build_tax_policy(profile), expected)

    def test_default_country_hooks(self):
        country = native.AbstractCountry("generic", "EUR")
        self.assertIsNone(country.validate_input_data([]))
        self.assertIsNone(country.compute_tax_for_assets(None, None, {}))

    def test_iso_4217_codes_match_rp2s_lookup(self):
        import pycountry

        self.assertEqual(native._ISO_4217, frozenset(currency.alpha_3 for currency in pycountry.currencies))

    def test_classification_without_an_engine_category_fails_closed(self):
        lot = object.__new__(native.GainLoss)
        lot._at_category = None
        lot._at_category_error = None
        with self.assertRaises(native.EngineRuntimeError):
            native.classify_disposal(lot)
        lot._at_category_error = {"class": "ValueError", "message": "Empty `at_swap_link=` marker"}
        with self.assertRaises(native.EngineValueError) as raised:
            native.classify_disposal(lot)
        self.assertEqual(str(raised.exception), "Empty `at_swap_link=` marker")
        with self.assertRaises(native.EngineTypeError):
            native.classify_disposal(object())


class ErrorClassTest(unittest.TestCase):
    def test_engine_errors_map_to_python_classes(self):
        cases = {
            "ValueError": native.EngineValueError,
            "TypeError": native.EngineTypeError,
            "RuntimeError": native.EngineRuntimeError,
            "InvalidOperation": __import__("decimal").InvalidOperation,
            "Unsupported": native.EngineContractError,
            "RequestError": native.EngineContractError,
        }
        for name, expected in cases.items():
            with self.subTest(name):
                error = native._engine_error({"class": name, "message": "m"})
                self.assertIsInstance(error, expected)
        # Like RP2's errors, they are not ValueError/TypeError subclasses.
        self.assertFalse(issubclass(native.EngineValueError, ValueError))
        self.assertEqual(str(native.EngineValueError("message")), "message")


class JournalProvenanceTest(unittest.TestCase):
    def test_calculation_metadata_names_the_backend(self):
        from kassiber.core.engines import GenericRP2TaxEngine
        from tests.custody_tax_helpers import finalized_tax_inputs
        from tests.test_tax_engine_parity import WALLET_REFS, _adapter_profile, _ledger_history

        profile = _adapter_profile("generic", "FIFO")
        rows, pairs = _ledger_history()
        inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
        expected, actual = compare.run_both(GenericRP2TaxEngine(profile), inputs)
        for (result, error), engine in ((expected, "rp2"), (actual, "kassiber_tax")):
            self.assertIsNone(error)
            names = {entry["calculation"]["engine"] for entry in result.entries if "calculation" in entry}
            self.assertEqual(names, {engine})


if __name__ == "__main__":
    unittest.main()

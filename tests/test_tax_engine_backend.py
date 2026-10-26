"""Production's isolation from RP2, and the binding.

Production journals run on ``KassiberTaxEngine`` and the Rust engine. These
tests pin what parity runs cannot show: that production runs with RP2
absent, writes nothing, opens no socket, and names the engine in its
journals, and that the binding keeps its versioned JSON contract: it exposes
the schema-2 ``ledger`` function only (the core keeps the schema-1
operations as its lot engine's fixture API).
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

from kassiber.core.engines import KassiberTaxEngine, errors, ledger_request, ledger_response
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
    # These children examine production's own process: no RP2, no files.
    # A shadow or census session's child hook would run the oracle there or
    # write route records, so they run without the session's tax mode.
    env.pop("KASSIBER_TEST_TAX_ENGINE", None)
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


def _capability_books_source() -> str:
    """Child-process source for ``BOOKS``: one book per engine capability.

    A generic book, a generic book with the Bitcoin rail carry, an Austrian
    book, and an Austrian book with a carrying-value swap, each a finalized
    projection of the generator's histories.
    """
    return textwrap.dedent(
        """
        from tests.custody_tax_helpers import finalized_tax_inputs
        from tests.tax_ledger_generator import WALLET_REFS, ledger_history, profile, rail_history

        def _book(country, method, history):
            book_profile = profile(country, method)
            rows, pairs = history
            return book_profile, finalized_tax_inputs(book_profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)

        BOOKS = {
            "generic": _book("generic", "FIFO", ledger_history()),
            "generic_carry": _book("generic", "hifo", rail_history()),
            "at": _book("at", "moving_average_at", ledger_history()),
            "at_swap": _book("at", "moving_average_at", rail_history()),
        }
        """
    )


class ProductionIsolationTest(unittest.TestCase):
    """No RP2 on the production path: production imports none of it and writes nothing."""

    def test_runs_journals_without_rp2_installed(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-norp2-") as scratch:
            script = _BLOCK_RP2 + textwrap.dedent(
                f"""
                import importlib.util
                import tempfile
                tempfile.tempdir = {scratch!r}

                # The block holds: RP2 and the packages only it brings in
                # cannot be imported here.
                for name in ("rp2", "prezzemolo", "pycountry"):
                    try:
                        __import__(name)
                    except ModuleNotFoundError:
                        pass
                    else:
                        raise AssertionError(name + " was importable")

                from kassiber.core.engines import KassiberTaxEngine
                from kassiber.tax_policy import build_tax_policy
                from tests.custody_tax_helpers import finalized_tax_inputs
                from tests.tax_ledger_generator import WALLET_REFS, ledger_history, profile, rail_history

                # An Austrian swap needs moving_average_at; under fifo it fails closed.
                books = (ledger_history(), rail_history())
                runs = (("generic", "FIFO", books), ("generic", "hifo", books), ("at", "fifo", books[:1]), ("at", "moving_average_at", books))
                for country, method, histories in runs:
                    book_profile = profile(country, method)
                    for rows, pairs in histories:
                        result = KassiberTaxEngine(book_profile).build_ledger_state(
                            finalized_tax_inputs(book_profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
                        )
                        realized = [entry for entry in result.entries if "calculation" in entry]
                        assert realized, (country, method)
                        assert {{entry["calculation"]["engine"] for entry in realized}} == {{"kassiber_tax"}}
                assert build_tax_policy({{"tax_country": "at"}}).default_accounting_method == "moving_average_at"

                loaded = sorted(name for name in sys.modules if name.split(".")[0] in {{"rp2", "prezzemolo", "pycountry"}})
                assert not loaded, loaded
                # The phase-3 adapter left the product at the cutover, and
                # its RP2-shaped mirror and request builders after it.
                for name in ("rp2", "native", "native_request"):
                    assert importlib.util.find_spec("kassiber.core.engines." + name) is None, name
                print("ok")
                """
            )
            result = _run_python(script, cwd=Path(scratch))
        self.assertEqual(result.returncode, 0, msg=f"stdout={result.stdout!r} stderr={result.stderr[-4000:]!r}")
        self.assertIn("ok", result.stdout)

    def test_writes_no_files_and_opens_no_sockets(self):
        # An audit hook records every write-mode open, file-system change and
        # socket call during production's journal runs, one book per engine
        # capability. The oracle's run on RP2 proves the hook sees its
        # temporary INI file.
        with tempfile.TemporaryDirectory(prefix="kassiber-engine-io-") as scratch:
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

                from kassiber.core.engines import KassiberTaxEngine
                from tests.tax_oracle.adapter import OracleTaxEngine
                """
            ) + _capability_books_source() + textwrap.dedent(
                """
                WRITE_FLAGS = os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_APPEND | os.O_TRUNC
                EVENTS = {
                    "os.mkdir", "os.remove", "os.rename", "os.rmdir", "os.truncate", "os.chdir",
                    "tempfile.mkstemp", "tempfile.mkdtemp", "socket.connect", "socket.getaddrinfo", "socket.bind",
                }
                recording = []
                state = {"active": False}

                def hook(event, args):
                    if not state["active"]:
                        return
                    if event == "open" and isinstance(args[2], int) and args[2] & WRITE_FLAGS:
                        recording.append([event, str(args[0])])
                    elif event in EVENTS:
                        recording.append([event, str(args[0]) if args else ""])

                sys.addaudithook(hook)
                runs = {name: (KassiberTaxEngine, book) for name, book in BOOKS.items()}
                runs["oracle on RP2"] = (OracleTaxEngine, BOOKS["generic"])
                for engine, (book_profile, inputs) in runs.values():  # warm up lazy imports
                    engine(book_profile).build_ledger_state(inputs)
                events = {}
                for name, (engine, (book_profile, inputs)) in runs.items():
                    recording.clear()
                    state["active"] = True
                    engine(book_profile).build_ledger_state(inputs)
                    state["active"] = False
                    events[name] = list(recording)
                print(json.dumps(events))
                """
            )
            result = _run_python(script, cwd=work)
            self.assertEqual(result.returncode, 0, msg=f"stderr={result.stderr[-4000:]!r}")
            events = json.loads(result.stdout.strip().splitlines()[-1])
            self.assertEqual(
                {name: events[name] for name in ("generic", "generic_carry", "at", "at_swap")},
                {"generic": [], "generic_carry": [], "at": [], "at_swap": []},
            )
            self.assertTrue(
                any(event == "open" and path.endswith(".ini") for event, path in events["oracle on RP2"]),
                events["oracle on RP2"],
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
    def test_engine_version_is_reported(self):
        self.assertRegex(kassiber_tax.engine_version(), r"^\d+\.\d+\.\d+")

    def test_the_binding_exposes_only_the_ledger_function(self):
        # The schema-1 operations stay in the core for the lot engine's
        # fixtures; Python has no caller for them.
        functions = sorted(
            name for name in dir(kassiber_tax) if not name.startswith("_") and callable(getattr(kassiber_tax, name))
        )
        self.assertEqual(functions, ["engine_version", "ledger"])

    def test_canonical_decimals_round_trip_beyond_the_int_conversion_limit(self):
        # Python caps int <-> str conversion at 4300 digits; the boundary form
        # must not, so a long but valid coefficient keeps its exact value.
        for value in (
            Decimal("1." + "0" * 5000),
            Decimal("-" + "9" * 5000 + "E-17"),
            Decimal("0E-28"),
            Decimal("-0.00"),
        ):
            with self.subTest(digits=len(value.as_tuple().digits)):
                encoded = ledger_request.encode_decimal(value)
                self.assertEqual(ledger_request.decode_decimal(encoded).as_tuple(), value.as_tuple())

    def test_malformed_requests_fail_as_request_errors(self):
        # A schema-1 document is another version, so ``ledger`` refuses it too.
        for text in ("not json", json.dumps({"schema_version": 1, "operation": "compute"})):
            with self.subTest(text=text):
                response = json.loads(kassiber_tax.ledger(text))
                self.assertFalse(response["ok"])
                self.assertEqual(response["error"]["stage"], "request")
                self.assertEqual(response["error"]["inner"]["class"], "RequestError")
                with self.assertRaises(errors.EngineContractError) as raised:
                    ledger_response.raise_for_error(response)
                self.assertEqual(raised.exception.error_class, "RequestError")

    def test_rust_panic_surfaces_as_runtime_error(self):
        # The binding converts a panic into RuntimeError, but it exposes no
        # entry point that panics on demand.
        self.skipTest("kassiber_tax exposes no way to trigger a panic")


class ErrorClassTest(unittest.TestCase):
    def test_engine_errors_map_to_python_classes(self):
        cases = {
            "ValueError": errors.EngineValueError,
            "TypeError": errors.EngineTypeError,
            "RuntimeError": errors.EngineRuntimeError,
            "InvalidOperation": __import__("decimal").InvalidOperation,
            "Unsupported": errors.EngineContractError,
            "RequestError": errors.EngineContractError,
        }
        for name, expected in cases.items():
            with self.subTest(name):
                error = errors.engine_error({"class": name, "message": "m"})
                self.assertIsInstance(error, expected)
        # Like RP2's errors, they are not ValueError/TypeError subclasses.
        self.assertFalse(issubclass(errors.EngineValueError, ValueError))
        self.assertEqual(str(errors.EngineValueError("message")), "message")
        self.assertEqual(str(errors.engine_error({"class": "Unsupported", "message": "m"})), "Unsupported: m")


class JournalProvenanceTest(unittest.TestCase):
    def test_calculation_metadata_names_the_backend(self):
        # Production's journals name the Rust engine on every capability; the
        # oracle's name RP2 (the comparison masks the field).
        from tests.custody_tax_helpers import finalized_tax_inputs
        from tests.tax_ledger_generator import WALLET_REFS, ledger_history, profile, rail_history

        def engines(result):
            return {entry["calculation"]["engine"] for entry in result.entries if "calculation" in entry}

        books = {
            "generic": ("generic", "FIFO", ledger_history()),
            "generic_carry": ("generic", "hifo", rail_history()),
            "at": ("at", "moving_average_at", ledger_history()),
            "at_swap": ("at", "moving_average_at", rail_history()),
        }
        # The call goes through the class, so a shadow session compares it
        # with the oracle. Under `oracle` the class runs the oracle in
        # production's place, so production is called through the harness's
        # unpatched method instead.
        session = os.environ.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower()
        for name, (country, method, (rows, pairs)) in books.items():
            with self.subTest(name):
                book_profile = profile(country, method)
                inputs = finalized_tax_inputs(book_profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
                self.assertEqual(compare.classify(book_profile, inputs), (name, compare.V2))
                if session == compare.ORACLE:
                    actual, error = compare.run_production(book_profile, inputs)
                    self.assertIsNone(error)
                else:
                    actual = KassiberTaxEngine(book_profile).build_ledger_state(inputs)
                self.assertEqual(engines(actual), {"kassiber_tax"})
                expected, error = compare.run_oracle(book_profile, inputs)
                self.assertIsNone(error)
                self.assertEqual(engines(expected), {"rp2"})


if __name__ == "__main__":
    unittest.main()

"""Unsupported-exchange onboarding: analyze a file, map its columns, import it.

The rules under test are the ones the feature's auditability rests on:

- An AI may map column names and label vocabulary. It may never supply a value.
  Every amount, date, and asset comes from the file through the deterministic
  importer, so a wrong plan produces a rejected row, not a wrong number.
- A plan from an untrusted source is validated against the file's real headers
  and the tax engine's real Types before it reaches the money path.
- When the AI provider does not run on this device, a file analysis carries no
  cell values at all — headers and counts are enough to propose a mapping.
"""

import base64
import tempfile
import unittest
from pathlib import Path

from kassiber import importers as importers_module
from kassiber.core import file_analysis
from kassiber.errors import AppError


# A German-labelled export from a platform with no dedicated importer: none of
# its column names or row values are Kassiber's own.
OPAQUE_CSV = (
    "Ausfuehrung,Richtung,Stueck,Spesen,Waehrung,Gegenwert\n"
    "2024-03-01T10:00:00Z,Kauf,0.5,0.0001,EUR,21000\n"
    "2024-04-02T10:00:00Z,Verkauf,0.25,0.0001,EUR,12000\n"
)
OPAQUE_PLAN = {
    "date": "Ausfuehrung",
    "type": "Richtung",
    "amount": "Stueck",
    "fee": "Spesen",
    "fiat_currency": "Waehrung",
    "fiat_value": "Gegenwert",
}


def _write(tmp, name, text):
    path = Path(tmp) / name
    path.write_text(text, encoding="utf-8")
    return str(path)


class ColumnPlanValidationTests(unittest.TestCase):
    """A plan decides which column is money, so it is a trust boundary."""

    HEADERS = ["Ausfuehrung", "Richtung", "Stueck", "Spesen"]

    def _refuse(self, plan):
        with self.assertRaises(AppError) as raised:
            file_analysis.normalize_column_map(plan, self.HEADERS)
        self.assertEqual(raised.exception.code, "validation")
        return raised.exception

    def test_accepts_a_plan_naming_real_columns(self):
        plan = file_analysis.normalize_column_map(
            {"date": "Ausfuehrung", "type": "Richtung", "amount": "Stueck"},
            self.HEADERS,
        )
        self.assertEqual(plan["date"], "Ausfuehrung")
        self.assertEqual(plan["amount"], "Stueck")

    def test_rejects_a_column_absent_from_the_file(self):
        # Otherwise a hallucinated column name silently becomes an empty
        # money field instead of an error.
        error = self._refuse({"date": "Ausfuehrung", "amount": "Volume"})
        self.assertIn("not in this file", str(error))

    def test_rejects_unknown_plan_fields(self):
        self._refuse({"date": "Ausfuehrung", "amount": "Stueck", "payout": "Spesen"})

    def test_rejects_one_column_claimed_by_two_fields(self):
        # Reading the same column as both amount and fee would double-count.
        error = self._refuse(
            {"date": "Ausfuehrung", "amount": "Stueck", "fee": "Stueck"}
        )
        self.assertIn("two different fields", str(error))

    def test_rejects_a_plan_with_no_date_or_no_amount(self):
        self._refuse({"amount": "Stueck"})
        self._refuse({"date": "Ausfuehrung"})

    def test_rejects_an_asset_hint_that_contradicts_the_column_header(self):
        # The dangerous case is a hint that is *valid but wrong*: relabelling a
        # column the file itself calls "BTC Amount" as SATS imports every row 1e8
        # too small, with no rejected row to notice. A plan maps names, not values.
        headers = ["Ausfuehrung", "BTC Amount"]
        for wrong in ("SATS", "LBTC", "EUR"):
            with self.subTest(hint=wrong):
                with self.assertRaises(AppError) as raised:
                    file_analysis.normalize_column_map(
                        {
                            "date": "Ausfuehrung",
                            "amount": "BTC Amount",
                            "amount_header_asset": wrong,
                        },
                        headers,
                    )
                self.assertIn("contradicts", str(raised.exception))

    def test_an_asset_hint_may_fill_in_a_silent_header(self):
        # Legitimate use: the column header says nothing about the asset.
        plan = file_analysis.normalize_column_map(
            {"date": "Ausfuehrung", "amount": "Stueck", "amount_header_asset": "LBTC"},
            ["Ausfuehrung", "Stueck"],
        )
        self.assertEqual(plan["amount_header_asset"], "LBTC")

    def test_rejects_an_asset_hint_kassiber_cannot_denominate(self):
        self._refuse(
            {
                "date": "Ausfuehrung",
                "amount": "Stueck",
                "amount_header_asset": "NOTANASSET",
            }
        )

    def test_rejects_a_type_map_inventing_a_tax_kind(self):
        # The vocabulary map may rename a Type; it may not create one. Otherwise
        # a model could route a disposal into a kind the engine never reviews.
        error = self._refuse(
            {
                "date": "Ausfuehrung",
                "type": "Richtung",
                "amount": "Stueck",
                "type_map": {"Kauf": "Tax Free Bonus"},
            }
        )
        self.assertIn("unknown ledger Type", str(error))

    def test_a_type_map_may_only_target_engine_recognized_types(self):
        plan = file_analysis.normalize_column_map(
            {
                "date": "Ausfuehrung",
                "type": "Richtung",
                "amount": "Stueck",
                "type_map": {"ACQ-MKT": "Buy"},
            },
            self.HEADERS,
        )
        # Keys are casefolded and whitespace-collapsed so the file's own casing
        # does not have to match; punctuation is significant.
        self.assertEqual(plan["type_map"], {"acq-mkt": "Buy"})

    def test_the_importer_itself_validates_an_unvalidated_plan(self):
        # The choke point is inside the importer, so no entry point can pass a
        # raw plan straight through to the row remapper.
        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "opaque.csv", OPAQUE_CSV)
            with self.assertRaises(AppError) as raised:
                importers_module.load_generic_ledger_records(
                    path, {"date": "Ausfuehrung", "amount": "NotAColumn"}
                )
            self.assertEqual(raised.exception.code, "validation")


class AnalyzeFileTests(unittest.TestCase):
    def test_unrecognized_headers_ask_for_a_plan_instead_of_guessing(self):
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "opaque.csv", OPAQUE_CSV)
            )
        self.assertFalse(analysis["confident"])
        self.assertEqual(analysis["next_step"]["action"], "supply_column_map")
        # The headers are handed back precisely so a plan can be built from them.
        self.assertIn("Ausfuehrung", analysis["headers"])
        self.assertEqual(analysis["mapped"], 0)

    def test_a_supplied_plan_makes_the_same_file_importable(self):
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "opaque.csv", OPAQUE_CSV), column_map=OPAQUE_PLAN
            )
        self.assertTrue(analysis["confident"])
        self.assertEqual(analysis["mapped"], 2)
        self.assertEqual(analysis["errors"], 0)
        self.assertEqual(analysis["next_step"]["action"], "import")
        # Values come from the file, not from any caller.
        first = analysis["preview"][0]
        self.assertEqual(first["amount"], "0.5")
        self.assertEqual(first["kind"], "buy")
        self.assertEqual(first["fiat_value"], "21000")

    def test_a_house_vocabulary_needs_a_type_map_and_is_not_guessed(self):
        csv_text = (
            "Trade Date,Action,Qty,Currency,Total\n"
            "2024-05-01T09:00:00Z,ACQ-MKT,1.5,EUR,60000\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "house.csv", csv_text)
            # Columns are recognized, but "ACQ-MKT" means nothing: the row is
            # rejected rather than booked as some default kind.
            guessed = file_analysis.analyze_file(path)
            self.assertEqual(guessed["mapped"], 0)
            self.assertEqual(guessed["errors"], 1)

            named = file_analysis.analyze_file(
                path,
                column_map={
                    "date": "Trade Date",
                    "type": "Action",
                    "amount": "Qty",
                    "fiat_currency": "Currency",
                    "fiat_value": "Total",
                    "type_map": {"ACQ-MKT": "Buy"},
                },
            )
        self.assertEqual(named["mapped"], 1)
        self.assertEqual(named["preview"][0]["kind"], "buy")

    def test_german_row_values_are_recognized_without_a_type_map(self):
        # German column names were already recognized; the row values now match,
        # so an Austrian export needs no AI round-trip at all.
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "opaque.csv", OPAQUE_CSV), column_map=OPAQUE_PLAN
            )
        kinds = [row["kind"] for row in analysis["preview"]]
        self.assertEqual(kinds, ["buy", "sell"])

    def test_a_german_direction_column_resolves_both_ways(self):
        # German *column* names were always recognized, so German *values* in a
        # direction column must be too, or the Austrian case half-works.
        csv_text = (
            "Datum,Richtung,Betrag,Währung,Wert\n"
            "2024-03-01T10:00:00Z,Eingang,0.5,EUR,21000\n"
            "2024-04-02T10:00:00Z,Ausgang,0.25,EUR,12000\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "dir.csv", csv_text))
        self.assertEqual(analysis["errors"], 0, analysis["problems"])
        self.assertEqual(
            [row["direction"] for row in analysis["preview"]],
            ["inbound", "outbound"],
        )

    def test_an_unrecognized_direction_value_is_refused_not_defaulted(self):
        # Regression: a mapped direction column whose value meant nothing fell
        # through to the "Deposit if inbound else Withdrawal" default, so every
        # such row was silently booked as a Withdrawal. An inbound row happened
        # to be caught by the leg-sign check; an outbound one would have been
        # accepted as a disposal on a guess.
        csv_text = (
            "Datum,Richtung,Betrag,Währung,Wert\n"
            "2024-03-01T10:00:00Z,ZZZ-UNKNOWN,0.5,EUR,21000\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "dir.csv", csv_text))
        self.assertEqual(analysis["mapped"], 0)
        self.assertEqual(analysis["errors"], 1)
        # Rejected by the offending cell's own value, so the user can map it.
        self.assertIn("ZZZ-UNKNOWN", analysis["problems"][0]["message"])

    def test_a_direction_value_that_names_a_tax_kind_is_still_refused(self):
        # Regression: an unknown direction value used to be routed through Type,
        # so a direction column reading "Income"/"Mining"/"Airdrop" booked that
        # tax kind on a guess instead of being rejected.
        for value in ("Income", "Mining", "Airdrop", "Gift", "ZZZ-UNKNOWN"):
            with self.subTest(direction=value):
                csv_text = (
                    "Datum,Richtung,Betrag,Waehrung,Wert\n"
                    f"2024-03-01T10:00:00Z,{value},0.5,EUR,21000\n"
                )
                with tempfile.TemporaryDirectory() as tmp:
                    analysis = file_analysis.analyze_file(
                        _write(tmp, "dir.csv", csv_text)
                    )
                self.assertEqual(analysis["mapped"], 0, analysis["preview"])
                self.assertEqual(analysis["errors"], 1)
                self.assertIn(
                    "unknown direction", analysis["problems"][0]["message"]
                )

    def test_a_german_type_column_is_recognized_so_a_sale_is_not_a_deposit(self):
        # Regression: "Typ" was missing from the Type aliases while German row
        # *values* were recognized, so an Austrian export half-worked in the
        # worst possible way — with no Type column the direction came from the
        # amount's sign alone, and an all-positive export booked "Verkauf" as an
        # inbound Deposit. Nothing was rejected: holdings doubled and a taxable
        # disposal disappeared.
        csv_text = (
            "Datum,Typ,Menge,Währung,Wert\n"
            "2024-03-01T10:00:00Z,Kauf,0.5,EUR,21000\n"
            "2024-04-02T10:00:00Z,Verkauf,0.25,EUR,12000\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "typ.csv", csv_text))
        self.assertEqual(analysis["errors"], 0, analysis["problems"])
        self.assertEqual(
            [(row["kind"], row["direction"]) for row in analysis["preview"]],
            [("buy", "inbound"), ("sell", "outbound")],
        )

    def test_a_file_with_no_type_or_direction_column_says_so(self):
        # With neither column every row's kind is the amount's sign, which for a
        # positive-only export means "everything is a deposit" with nothing
        # rejected. The file may genuinely be transfers only, so this is
        # reported rather than refused — but it must never read as "import".
        csv_text = (
            "Datum,Menge,Währung,Wert\n"
            "2024-03-01T10:00:00Z,0.5,EUR,21000\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "flat.csv", csv_text))
        self.assertTrue(analysis["row_kinds_from_amount_sign"])
        self.assertEqual(analysis["next_step"]["action"], "map_row_kinds")

    def test_a_mapped_type_column_clears_the_amount_sign_warning(self):
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "opaque.csv", OPAQUE_CSV), column_map=OPAQUE_PLAN
            )
        self.assertFalse(analysis["row_kinds_from_amount_sign"])
        self.assertEqual(analysis["next_step"]["action"], "import")

    def test_an_empty_direction_cell_is_refused_as_a_direction(self):
        # Regression: a blank cell in a mapped direction column skipped the
        # unreadable-direction check and was rejected downstream as a Type
        # mismatch ("Type 'Withdrawal' is outbound but the leg is inbound"),
        # sending the user after a Type the file never had.
        csv_text = "Datum,Richtung,Betrag\n2024-03-01T10:00:00Z,,0.5\n"
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "blank.csv", csv_text),
                column_map={"date": "Datum", "direction": "Richtung", "amount": "Betrag"},
            )
        self.assertEqual(analysis["mapped"], 0)
        self.assertIn("direction", analysis["problems"][0]["message"])

    def test_amounts_too_large_to_be_btc_are_not_imported_as_btc(self):
        # Nothing in this file says what "Amount" holds, so the importer falls
        # back to BTC and 100000 becomes 100,000 BTC instead of 0.001 — a 1e8
        # error with no rejected row, since every value is individually valid.
        csv_text = "Date,Type,Amount\n2024-01-01T10:00:00Z,Buy,100000\n"
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "sats.csv", csv_text))
        self.assertTrue(analysis["amount_units_unconfirmed"])
        self.assertEqual(analysis["next_step"]["action"], "confirm_amount_units")

    def test_btc_shaped_amounts_do_not_ask_about_units(self):
        # The signal has to stay quiet on the normal case, or it is ignored:
        # a silent header with fractional amounts is unambiguously BTC-shaped.
        csv_text = "Date,Type,Amount\n2024-01-01T10:00:00Z,Buy,0.5\n"
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "btc.csv", csv_text))
        self.assertFalse(analysis["amount_units_unconfirmed"])
        self.assertEqual(analysis["next_step"]["action"], "import")

    def test_a_column_that_states_its_asset_is_believed(self):
        csv_text = "Date,Type,SATS Amount\n2024-01-01T10:00:00Z,Buy,100000\n"
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "sats.csv", csv_text))
        self.assertFalse(analysis["amount_units_unconfirmed"])
        self.assertEqual(analysis["preview"][0]["asset"], "BTC")
        # 100000 sats, not 100000 BTC.
        self.assertEqual(analysis["preview"][0]["amount"], "0.001")

    def test_photos_and_pdfs_route_to_the_on_device_document_importer(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "statement.pdf"
            path.write_bytes(b"%PDF-1.4 not really a pdf")
            analysis = file_analysis.analyze_file(str(path))
        self.assertEqual(analysis["route"], "document_import")
        self.assertEqual(analysis["next_step"]["action"], "document_import_preview")

    def test_refuses_a_file_type_it_cannot_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "wallet.dat", "binary-ish")
            with self.assertRaises(AppError) as raised:
                file_analysis.analyze_file(path)
        self.assertEqual(raised.exception.code, "validation")

    def test_missing_file_is_not_found_not_a_crash(self):
        with self.assertRaises(AppError) as raised:
            file_analysis.analyze_file("/nonexistent/nope.csv")
        self.assertEqual(raised.exception.code, "not_found")


class EgressRedactionTests(unittest.TestCase):
    """Header-only is the whole privacy story for off-device providers."""

    def _analysis(self, tmp):
        return file_analysis.analyze_file(
            _write(tmp, "opaque.csv", OPAQUE_CSV), column_map=OPAQUE_PLAN
        )

    def test_no_cell_value_survives_redaction(self):
        with tempfile.TemporaryDirectory() as tmp:
            redacted = file_analysis.redact_for_egress(self._analysis(tmp))
            flattened = repr(redacted)
            for leaked in ("0.5", "21000", "0.0001", "2024-03-01", tmp):
                self.assertNotIn(leaked, flattened, f"{leaked} reached the model")

    def test_what_a_model_needs_to_propose_a_plan_does_survive(self):
        with tempfile.TemporaryDirectory() as tmp:
            redacted = file_analysis.redact_for_egress(self._analysis(tmp))
        self.assertEqual(
            redacted["headers"],
            ["Ausfuehrung", "Richtung", "Stueck", "Spesen", "Waehrung", "Gegenwert"],
        )
        self.assertEqual(redacted["rows_read"], 2)
        self.assertTrue(redacted["cell_values_withheld"])
        self.assertIn("next_step", redacted)

    def test_rejected_row_messages_are_reduced_to_a_count(self):
        # Per-row validation messages quote the offending cell, so they are
        # summarized rather than forwarded.
        csv_text = (
            "Trade Date,Action,Qty\n"
            "2024-05-01T09:00:00Z,SECRET-CODE-9,1.5\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "h.csv", csv_text))
            self.assertEqual(analysis["errors"], 1)
            redacted = file_analysis.redact_for_egress(analysis)
        self.assertEqual(redacted["problem_rows"], 1)
        self.assertNotIn("problems", redacted)
        self.assertNotIn("SECRET-CODE-9", repr(redacted))

    def test_the_local_path_is_never_forwarded(self):
        with tempfile.TemporaryDirectory() as tmp:
            analysis = self._analysis(tmp)
            self.assertIn("path", analysis["source"])  # local callers keep it
            redacted = file_analysis.redact_for_egress(analysis)
        self.assertNotIn("path", redacted["source"])


class HeaderEgressTests(unittest.TestCase):
    """A "header" is only safe to disclose if the file actually has one."""

    def _redacted_headers(self, body):
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(_write(tmp, "export.csv", body))
            return file_analysis.redact_for_egress(analysis)

    def test_a_preamble_line_is_withheld_not_sent_as_column_names(self):
        # `analyze_file` takes the first non-empty row; for a bank-style export
        # that row is an account holder, an IBAN and a date range.
        redacted = self._redacted_headers(
            "Kontoauszug Max Mustermann AT611904300234573201 01.01.2024-31.12.2024\n"
            "Datum,Richtung,Betrag\n"
            "2024-03-01,Eingang,0.5\n"
        )
        flattened = repr(redacted)
        for leaked in ("Mustermann", "AT611904300234573201", "01.01.2024"):
            self.assertNotIn(leaked, flattened)
        self.assertEqual(redacted["headers_withheld"], 1)

    def test_an_all_text_preamble_is_withheld_too(self):
        # Regression: the content heuristic only fires on a cell that *looks
        # like* a value, so "Account holder,Max Mustermann" read as two
        # plausible column names and a real person's name was sent off-device.
        # Nothing in the text separates it from a header — the shape does: a
        # header names every column, so it is never narrower than its rows.
        redacted = self._redacted_headers(
            "Account holder,Max Mustermann\n"
            "Date,Type,Amount,Currency\n"
            "2024-03-01,Buy,0.5,EUR\n"
        )
        self.assertNotIn("Mustermann", repr(redacted))
        self.assertEqual(redacted["headers_withheld"], 2)
        # ...and the user is told why the file cannot be mapped as it stands.
        self.assertEqual(redacted["next_step"]["action"], "strip_preamble")

    def test_a_headerless_export_discloses_no_cell(self):
        redacted = self._redacted_headers(
            "2024-03-01,Kauf,0.5,Coinfinity,"
            + "de" * 32
            + "\n"
        )
        flattened = repr(redacted)
        # Including the text cells: a counterparty is indistinguishable from a
        # column name in isolation, so the whole data row is withheld.
        for leaked in ("2024-03-01", "0.5", "Coinfinity", "dede"):
            self.assertNotIn(leaked, flattened)

    def test_a_real_header_row_is_disclosed_intact(self):
        redacted = self._redacted_headers(
            "Datum,Richtung,Betrag,Waehrung,Wert\n2024-03-01,Eingang,0.5,EUR,21000\n"
        )
        self.assertEqual(
            redacted["headers"],
            ["Datum", "Richtung", "Betrag", "Waehrung", "Wert"],
        )
        self.assertEqual(redacted["headers_withheld"], 0)

    def test_a_personal_filename_does_not_travel(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "Umsatzliste_AT61_max.mustermann.csv"
            path.write_text("Datum,Betrag\n2024-03-01,0.5\n", encoding="utf-8")
            redacted = file_analysis.redact_for_egress(
                file_analysis.analyze_file(str(path))
            )
        self.assertNotIn("mustermann", repr(redacted).lower())
        # The extension is the useful part and is kept.
        self.assertEqual(redacted["source"]["extension"], ".csv")


class DaemonAnalyzeFileTests(unittest.TestCase):
    def test_uploaded_bytes_analyze_without_a_caller_supplied_path(self):
        from kassiber.daemon import _analyze_file_payload

        payload = _analyze_file_payload(
            None,
            {
                "filename": "xyz-export.csv",
                "source_bytes_base64": base64.b64encode(
                    OPAQUE_CSV.encode()
                ).decode(),
                "column_map": OPAQUE_PLAN,
            },
        )
        self.assertEqual(payload["mapped"], 2)
        # The temp path is already deleted; handing it back would imply it is
        # importable.
        self.assertIsNone(payload["source"]["path"])
        self.assertEqual(payload["source"]["filename"], "xyz-export.csv")

    def test_a_caller_supplied_path_is_refused(self):
        from kassiber.daemon import _analyze_file_payload

        with tempfile.TemporaryDirectory() as tmp:
            secret = _write(tmp, "secret.env", "API_TOKEN=super_secret_value\n")
            with self.assertRaises(AppError) as raised:
                _analyze_file_payload(None, {"source_file": secret})
        self.assertIn("token or source_bytes_base64 is required", str(raised.exception))
        self.assertNotIn("super_secret_value", str(raised.exception))

    def test_unsupported_upload_extension_is_refused(self):
        from kassiber.daemon import _analyze_file_payload

        with self.assertRaises(AppError) as raised:
            _analyze_file_payload(
                None,
                {
                    "filename": "wallet.dat",
                    "source_bytes_base64": base64.b64encode(b"x").decode(),
                },
            )
        self.assertEqual(raised.exception.code, "validation")


class AiToolBoundaryTests(unittest.TestCase):
    """What the model may and may not hand the analysis tool."""

    def _runtime(self, tmp, path, **state):
        from kassiber.daemon import AiToolRuntime
        import queue

        return AiToolRuntime(
            data_root=tmp,
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={"attachment_source_file": path, **state},
        )

    def test_the_model_cannot_declare_what_a_column_is_denominated_in(self):
        # An asset hint is a value, not a name: SATS on a BTC column imports
        # every row 1e8 too small. Those stay with the human --column-map.
        from kassiber.daemon import _analyze_attached_file_for_ai

        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "opaque.csv", OPAQUE_CSV)
            runtime = self._runtime(tmp, path, provider_on_device=True)
            for field in importers_module.LEDGER_ASSET_HINT_FIELDS:
                with self.subTest(field=field):
                    with self.assertRaises(AppError) as raised:
                        _analyze_attached_file_for_ai(
                            runtime, {"column_map": {**OPAQUE_PLAN, field: "SATS"}}
                        )
                    self.assertIn("Asset hints", str(raised.exception))

    def test_sample_rows_is_bounded_and_typed_at_the_boundary(self):
        from kassiber.daemon import _analyze_sample_rows_arg

        self.assertEqual(_analyze_sample_rows_arg({}), file_analysis.DEFAULT_SAMPLE_ROWS)
        # 0 must mean zero rows, not "unbounded" — otherwise a caller can pull one
        # problem message per bad row into its context.
        self.assertEqual(_analyze_sample_rows_arg({"sample_rows": 0}), 0)
        self.assertEqual(
            _analyze_sample_rows_arg({"sample_rows": 10_000}),
            file_analysis.MAX_SAMPLE_ROWS,
        )
        with self.assertRaises(AppError):
            _analyze_sample_rows_arg({"sample_rows": "lots"})

    def test_zero_sample_rows_returns_no_problem_messages(self):
        # The bound applies to `problems` too, which quote offending cells.
        csv_text = "Trade Date,Action,Qty\n" + "".join(
            f"2024-05-0{i},SECRET-{i},1.5\n" for i in range(1, 6)
        )
        with tempfile.TemporaryDirectory() as tmp:
            analysis = file_analysis.analyze_file(
                _write(tmp, "h.csv", csv_text), sample_rows=0
            )
        self.assertEqual(analysis["errors"], 5)
        self.assertEqual(analysis["problems"], [])
        self.assertNotIn("SECRET", repr(analysis))


class ProviderLocalityTests(unittest.TestCase):
    """`kind == "local"` is not the same question as "runs on this device"."""

    def test_only_loopback_local_providers_count_as_on_device(self):
        from kassiber.daemon import _provider_is_on_device

        for base_url, expected in (
            ("http://localhost:11434/v1", True),
            ("http://127.0.0.1:11434/v1", True),
            ("http://[::1]:11434/v1", True),
            # A "local" row can still point off-device; cell values must not go.
            ("http://192.168.1.50:11434/v1", False),
            ("https://ollama.example.com/v1", False),
            ("", False),
        ):
            with self.subTest(base_url=base_url):
                self.assertEqual(
                    _provider_is_on_device({"kind": "local", "base_url": base_url}),
                    expected,
                )

    def test_remote_provider_kinds_are_never_on_device(self):
        from kassiber.daemon import _provider_is_on_device

        self.assertFalse(
            _provider_is_on_device(
                {"kind": "anthropic", "base_url": "http://localhost:11434/v1"}
            )
        )


class AttachmentTurnNoteTests(unittest.TestCase):
    """The note announcing the attachment is prompt text, so it is egress too."""

    def _note(self, *, on_device):
        from kassiber.daemon import _attachment_context_for_model

        return _attachment_context_for_model(
            {
                "attachment_filename": "Umsatzliste_Max.Mustermann_AT61.csv",
                "attachment_label": "2024 export",
                "provider_on_device": on_device,
            }
        )

    def test_a_remote_model_is_not_told_the_filename(self):
        # Regression: `redact_for_egress` drops `filename` from the tool result
        # because it is routinely personal — and the turn note handed the very
        # same string to the provider on every attached turn, off-device
        # included. The extension is the only part a column plan needs.
        note = self._note(on_device=False)
        self.assertNotIn("Mustermann", note)
        self.assertNotIn("AT61", note)
        self.assertIn(".csv", note)
        # The user's own description is theirs to send; it stays.
        self.assertIn("2024 export", note)

    def test_an_on_device_model_may_be_told_the_filename(self):
        self.assertIn(
            "Umsatzliste_Max.Mustermann_AT61.csv", self._note(on_device=True)
        )


class ChatAttachmentContractTests(unittest.TestCase):
    def test_an_attachment_must_be_a_token_never_a_path(self):
        from kassiber.daemon import _ai_chat_attachment

        self.assertIsNone(_ai_chat_attachment(None))
        grant = _ai_chat_attachment({"token": " abc ", "label": " Kraken export "})
        self.assertEqual(grant, {"token": "abc", "label": "Kraken export"})

        for bad in (
            {"source_file": "/etc/passwd"},
            {"token": ""},
            {"token": "abc", "path": "/etc/passwd"},
            {"token": "abc", "label": "x" * 501},
            "abc",
        ):
            with self.subTest(bad=bad):
                with self.assertRaises(AppError):
                    _ai_chat_attachment(bad)

    def test_the_ai_tool_cannot_name_a_file(self):
        from kassiber.ai.tools import get_tool

        entry = get_tool("ui.wallets.analyze_file")
        self.assertIsNotNone(entry)
        self.assertEqual(entry.kind_class, "read_only")
        properties = set(entry.parameters["properties"])
        self.assertEqual(properties, {"column_map", "sample_rows"})
        self.assertFalse(entry.parameters["additionalProperties"])

    def test_the_tool_reports_a_missing_attachment_rather_than_reading_anything(self):
        from kassiber.daemon import _analyze_attached_file_for_ai, AiToolRuntime
        import queue

        runtime = AiToolRuntime(
            data_root="/tmp",
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={},
        )
        with self.assertRaises(AppError) as raised:
            _analyze_attached_file_for_ai(runtime, {})
        self.assertEqual(raised.exception.code, "ai_attachment_missing")

    def test_an_off_device_provider_gets_no_cell_values_from_the_tool(self):
        from kassiber.daemon import _analyze_attached_file_for_ai, AiToolRuntime
        import queue

        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "opaque.csv", OPAQUE_CSV)
            state = {
                "attachment_source_file": path,
                "attachment_label": "export from exchange XYZ, custodial",
            }
            remote = _analyze_attached_file_for_ai(
                AiToolRuntime(
                    data_root=tmp,
                    runtime_config={},
                    main_thread_tasks=queue.Queue(),
                    maintenance_state={**state, "provider_on_device": False},
                ),
                {"column_map": OPAQUE_PLAN},
            )
            local = _analyze_attached_file_for_ai(
                AiToolRuntime(
                    data_root=tmp,
                    runtime_config={},
                    main_thread_tasks=queue.Queue(),
                    maintenance_state={**state, "provider_on_device": True},
                ),
                {"column_map": OPAQUE_PLAN},
            )

        self.assertTrue(remote["cell_values_withheld"])
        self.assertNotIn("21000", repr(remote))
        # The user's own description of the file is context they typed, so it
        # travels with either provider.
        self.assertEqual(remote["user_description"], state["attachment_label"])
        # On-device, the model may see rows — but still not the path.
        self.assertEqual(local["preview"][0]["fiat_value"], "21000")
        self.assertNotIn("path", local["source"])


if __name__ == "__main__":
    unittest.main()

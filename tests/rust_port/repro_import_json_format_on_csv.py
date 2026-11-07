"""Check a JSON-format file import of a CSV file is a typed refusal.

    uv run --locked python -m tests.rust_port.repro_import_json_format_on_csv

Wallet onboarding found that `ui.wallets.import_file` with `source_format`
`json` on a CSV export crashed with a `JSONDecodeError` traceback, because
`load_import_records` ran `json.load` unguarded. The BTCPay JSON and Wasabi
bundle loaders did the same, and a non-UTF-8 file raised `UnicodeDecodeError`.
Each now refuses with `validation`. Exit 0 means fixed. Scratch stays under
target/.
"""
from pathlib import Path
import tempfile

from kassiber.errors import AppError
from kassiber.importers import load_import_records


def main():
    scratch = Path(__file__).resolve().parents[2] / "target/import-json-on-csv-repro"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch.resolve()) as temporary:
        csv_file = Path(temporary) / "export.csv"
        csv_file.write_text("date,txid,direction,asset,amount\n2025-01-02,ab,inbound,BTC,0.1\n")
        latin = Path(temporary) / "latin.json"
        latin.write_bytes(b'[{"description": "caf\xe9"}]')
        for path in (csv_file, latin):
            for input_format in ("json", "btcpay_json", "wasabi_bundle"):
                try:
                    load_import_records(str(path), input_format)
                except AppError as error:
                    assert error.code == "validation", (input_format, error.code)
                    print(f"{path.name} as {input_format}: {error.code}: {error}")
                else:
                    raise SystemExit(f"not refused: {path.name} as {input_format}")


if __name__ == "__main__":
    main()

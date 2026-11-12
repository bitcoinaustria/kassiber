"""Reproduce the Python Blockpit reader's bare-CR record refusal offline."""

import csv
import json
from pathlib import Path

from kassiber.importers import load_import_records


def main():
    root = Path(__file__).resolve().parents[2] / "target/blockpit-cr-repro"
    root.mkdir(parents=True, exist_ok=True)
    text = (
        "Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount\r"
        "2025-01-02,Source,Deposit,,,BTC,0.001\r"
    )
    source = root / "bare-cr.csv"
    source.write_bytes(text.encode())
    try:
        load_import_records(str(source), "blockpit_csv")
    except csv.Error as exc:
        error = str(exc)
    else:
        raise AssertionError("Python now accepts bare CR; review the frozen refusal")
    assert error == (
        "new-line character seen in unquoted field - "
        "do you need to open the file with newline=''?"
    ), error
    equivalent = root / "crlf.csv"
    equivalent.write_bytes(text.replace("\r", "\r\n").encode())
    records = load_import_records(str(equivalent), "blockpit_csv")
    assert len(records) == 1, records
    print(json.dumps({"bare_cr_error": error, "equivalent_crlf_records": len(records)}))


if __name__ == "__main__":
    main()

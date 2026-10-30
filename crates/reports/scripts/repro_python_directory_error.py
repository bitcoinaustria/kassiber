"""Reproduce Python's random staging path in CSV directory-target errors.

Run from the repository root with the locked environment. No product files or
books are touched; scratch lives under target and is removed on completion.
"""
from pathlib import Path
import json
import tempfile

from kassiber.core.reports import _write_csv_rows


def main():
    scratch = Path("target/r5-report-directory-repro")
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as temporary:
        target = Path(temporary) / "directory"
        target.mkdir()
        messages = []
        for _ in range(2):
            try:
                _write_csv_rows(target, [["header"], ["value"]])
            except IsADirectoryError as error:
                assert error.filename2 == str(target.resolve())
                assert Path(error.filename).name.startswith(".directory.")
                messages.append(str(error))
            else:
                raise AssertionError("expected directory target to fail")
        assert messages[0] != messages[1], messages
        assert list(target.iterdir()) == []
        assert list(target.parent.iterdir()) == [target]
        print(json.dumps({"reference_bug": "random CSV staging filename in error", "errors": messages}, indent=2))
    scratch.rmdir()


if __name__ == "__main__":
    main()

"""Freeze the reference's platform vocabulary and workbook column contracts."""
import json
from pathlib import Path
import sys

from kassiber import importers
from kassiber.core import cross_check

OUT = Path(__file__).resolve().parents[1] / 'src/cross_check_metadata.json'


def build():
    return dict(
        fiat=sorted(importers.FIAT_CURRENCIES),
        cointracking_types=importers._COINTRACKING_TYPES,
        blockpit_types=importers._BLOCKPIT_TYPES,
        generic_types=importers._GENERIC_LEDGER_TYPES,
        cointracking_required=importers._COINTRACKING_REQUIRED_COLUMNS,
        blockpit_required=importers._BLOCKPIT_REQUIRED_COLUMNS,
        columns=cross_check._ROW_COLUMNS,
        sheets=cross_check._SHEETS,
    )


if __name__ == '__main__':
    content = json.dumps(build(), ensure_ascii=True, indent=2) + '\n'
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'cross-check metadata drifted'
    else:
        OUT.write_text(content)

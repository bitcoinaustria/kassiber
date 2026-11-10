#!/usr/bin/env python3
"""Freeze report presentation constants and Vienna's TZif transitions; no books."""
from pathlib import Path
from datetime import datetime, timezone
import json
import sys
from zoneinfo import _common, _tzpath
from kassiber.core import reports

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'crates/reports/src/austrian_metadata.json'

def build():
    constants = {name: value for name, value in vars(reports).items()
                 if name.startswith('AUSTRIAN_') and isinstance(value, (str, dict, tuple))}
    with open(_tzpath.find_tzfile('Europe/Vienna'), 'rb') as source:
        idx, times, offsets, isdst, abbreviations, tail = _common.load_data(source)
    assert tail == b'CET-1CEST,M3.5.0,M10.5.0/3', tail
    epoch = datetime(1, 1, 1, tzinfo=timezone.utc)
    unix = (datetime(1970, 1, 1, tzinfo=timezone.utc) - epoch).days * 86400
    constants['vienna_transitions'] = [[0, offsets[0]], *[[(t + unix) * 1000000, offsets[i]] for t, i in zip(times, idx)]]
    empty = {'sections': reports._austrian_tax_sections([])}
    constants['section_tables'] = reports._austrian_e1kv_section_table_specs(empty)
    constants['overview'] = reports._austrian_e1kv_overview_entries(empty)
    from kassiber.core.austrian import tax_year_in_vienna, vienna_local_date
    dates = ['1800-01-01T23:00:00Z','1893-03-31T23:00:00Z','1916-05-01T21:59:59Z',
             '1945-04-02T21:59:59Z','1980-04-05T22:30:00Z','2024-12-31T23:30:00Z',
             '2024-03-31T21:59:59Z','2024-03-31T22:00:00Z','2024-10-27T22:00:00Z',
             '2038-03-28T22:00:00Z','2038-10-31T22:00:00Z','2100-03-28T22:00:00Z',
             '2100-10-31T22:00:00Z','2025-01-01T00:30:00+02:00','20241231T233000Z',
             '2024-W01-1T23:00:00Z','2024-12-31T23:00:00','2024-12-31']
    constants['calendar_vectors'] = [[raw,tax_year_in_vienna(raw),vienna_local_date(raw)] for raw in dates]
    return json.dumps(constants, ensure_ascii=False, indent=2) + '\n'

if __name__ == '__main__':
    content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'Austrian metadata drifted'
    else:
        OUT.write_text(content)

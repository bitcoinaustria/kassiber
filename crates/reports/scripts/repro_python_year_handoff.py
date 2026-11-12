from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import _report_hooks
from kassiber.core.year_handoff import build_year_handoff
from kassiber.db import open_db
from kassiber.errors import AppError


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(dir=ROOT / 'target/scratch') as raw:
        conn = open_db(Path(raw) / 'data')
        conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text())
        conn.executescript("DELETE FROM journal_quarantines; UPDATE profiles SET tax_country=' at ' WHERE id='p';")
        hooks = replace(_report_hooks(), now_iso=lambda: '2026-01-02T03:04:05Z')
        spaced = build_year_handoff(conn, None, None, hooks, year=2024)
        conn.execute("UPDATE profiles SET tax_country='at' WHERE id='p'")
        clean = build_year_handoff(conn, None, None, hooks, year=2024)
        conn.executescript("UPDATE profiles SET tax_country='generic' WHERE id='p'; INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59.5Z',77777,'77777','manual','fixed');")
        fraction = build_year_handoff(conn, None, None, hooks, year=2024)
        row = next(r for r in fraction['balances'] if r['asset'] == 'BTC')
        result = dict(spaced_country_calendar=spaced['period'].calendar,
            clean_country_calendar=clean['period'].calendar,
            final_fractional_second_quote=77777,
            actual_rate=row['year_end_rate'], actual_rate_source=row['rate_source'])
        assert result['spaced_country_calendar'] == 'UTC'
        assert result['clean_country_calendar'] == 'Europe/Vienna'
        assert result['actual_rate'] != 77777
        assert result['actual_rate_source'] == 'transaction price'
        try:
            build_year_handoff(conn, None, None, hooks, year=None)
        except AppError as error:
            result['missing_year_message'] = str(error)
        assert 'Austrian E 1kv' in result['missing_year_message']
        print(json.dumps(result, sort_keys=True))
        conn.close()

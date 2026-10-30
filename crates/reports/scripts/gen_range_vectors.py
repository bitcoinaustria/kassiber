#!/usr/bin/env python3
"""Freeze CPython zoneinfo offsets and export-range resolution.

The TZif files are copied once from the system tzdata into the fixture
directory; both CPython (via reset_tzpath) and the Rust test read only
those copies, so the vectors do not depend on the host tzdata version.
"""
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
import json
import shutil
import sys
import zoneinfo

from kassiber.core import export_range as ranges
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'crates/reports/tests/data/ranges.json'
ZONES_DIR = ROOT / 'crates/reports/tests/data/zoneinfo'
ZONES = (
    'UTC', 'Europe/Vienna', 'Europe/Dublin', 'Africa/Casablanca', 'America/Santiago',
    'America/Havana', 'America/Asuncion', 'Asia/Beirut', 'Pacific/Apia', 'Pacific/Kiritimati',
    'Australia/Lord_Howe', 'America/Nuuk', 'Asia/Tehran', 'Asia/Kathmandu', 'Pacific/Chatham',
    'America/St_Johns', 'Antarctica/Troll', 'Europe/London', 'America/New_York', 'Asia/Gaza',
    'Etc/GMT+12', 'Etc/GMT-14', 'EST5EDT', 'America/Scoresbysund', 'right/UTC',
)
YEARS = (1880, 1916, 1945, 1996, 2024, 2038, 2041, 2087)
UTC = timezone.utc


def offsets(zone):
    local, utc = {}, {}
    for year in YEARS:
        day = date(year, 1, 1)
        local[year], utc[year] = [], []
        while day.year == year:
            midnight = datetime(day.year, day.month, day.day, tzinfo=zone)
            local[year].append(int(midnight.utcoffset().total_seconds()))
            for hour in (0, 1, 23):
                instant = datetime(day.year, day.month, day.day, hour, 30, tzinfo=UTC)
                utc[year].append(int(instant.astimezone(zone).utcoffset().total_seconds()))
            day += timedelta(days=1)
    def runs(values):
        encoded = []
        for value in values:
            if encoded and encoded[-1][0] == value:
                encoded[-1][1] += 1
            else:
                encoded.append([value, 1])
        return encoded
    return {'local': {y: runs(v) for y, v in local.items()}, 'utc': {y: runs(v) for y, v in utc.items()}}


def build(copy):
    if copy:
        shutil.rmtree(ZONES_DIR, ignore_errors=True)
        for key in ZONES:
            target = ZONES_DIR / key
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path('/usr/share/zoneinfo') / key, target)
    zoneinfo.reset_tzpath([str(ZONES_DIR)])
    zoneinfo.ZoneInfo.clear_cache()
    result = {'years': YEARS, 'zones': {key: offsets(zoneinfo.ZoneInfo(key)) for key in ZONES}}
    cases = []

    def add(name, now='2024-06-15T12:00:00Z', country='generic', **kwargs):
        cases.append(dict(name=name, now=now, country=country, preset=kwargs.get('preset'),
                          first=kwargs.get('first'), last=kwargs.get('last'), timezone=kwargs.get('timezone')))

    for preset in (None, 'all', ' ALL ', '', '  '):
        add(f'all-time-{preset!r}', preset=preset)
    add('timezone-without-range', timezone='Mars/Phobos')
    add('preset-and-day', preset='today', first='2024-01-01')
    add('all-and-day', preset=' ALL ', last='2024-01-01')
    add('unknown-preset', preset='fortnight')
    add('unknown-preset-bad-zone', preset='fortnight', timezone='Mars/Phobos')
    add('custom-without-days', preset='custom')
    add('custom-without-days-bad-zone', preset=' Custom ', timezone='bad zone')
    add('only-from', first='2024-01-01')
    add('only-to', last='2024-01-01', timezone='Asia/Kathmandu')
    for text in ('2025-1-01', '２０２５-０１-０１', '2025-02-30', '0000-01-01', '2025-01-01T00:00', ' 2025-01-01',
                 '2024-02-29', '2023-02-29', '9999-12-31x'):
        add(f'from-{text}', first=text, last='2025-12-31')
        add(f'to-{text}', first='2024-01-01', last=text)
    add('ends-before-start', first='2025-01-02', last='2025-01-01')
    for country in ('at', 'AT', ' at', None, 'generic', ''):
        add(f'book-zone-{country!r}', country=country, first='2024-03-31', last='2024-10-27')
    # 'Europe' (a tzdata directory) is a reviewed difference: CPython leaks an
    # IsADirectoryError with its install path, Rust answers Unknown timezone.
    for key in ZONES + ('', 'europe/vienna', '../x', '/etc/passwd', 'Europe/Vienna/', 'UTC ',
                        'Europe//Vienna', './UTC', '../zoneinfo/UTC', 'zone.tab', 'a\x00b', 'Mars/Phobos'):
        add(f'zone-{key!r}', timezone=key, first='2024-03-31', last='2024-10-27')
    for first, last in (('2025-01-01', '2025-12-31'), ('2024-02-01', '2024-02-29'), ('2025-02-01', '2025-02-28'),
                        ('2025-04-01', '2025-06-30'), ('2025-02-01', '2025-04-30'), ('2025-03-01', '2025-05-31'),
                        ('2025-05-05', '2025-05-05'), ('2025-05-05', '2025-06-04'), ('2024-01-01', '2025-12-31'),
                        ('0999-02-01', '0999-02-28'), ('0005-01-01', '0005-12-31'), ('9999-12-01', '9999-12-30'),
                        ('9999-10-01', '9999-12-30'), ('9999-12-31', '9999-12-31'), ('0001-01-01', '0001-01-31'),
                        ('2024-12-30', '2025-01-05')):
        for key in ('UTC', 'Europe/Vienna', 'Etc/GMT+12'):
            add(f'label-{first}-{last}-{key}', timezone=key, first=first, last=last)
    nows = ('2024-12-31T23:30:00Z', '2024-12-31T22:59:59Z', '2025-01-01T00:00:00Z', '2024-02-29T12:00:00Z',
            '2024-03-31T00:30:00Z', '2024-10-27T00:30:00Z', '2024-06-30T22:30:00Z', '2024-09-30T23:00:00Z',
            '2024-01-01T00:00:00Z', '2025-03-30T01:00:00Z', '2025-06-15T23:59:59Z', '2023-01-01T11:59:59Z',
            '2087-06-01T00:00:00Z', '2040-12-31T23:59:59Z')
    for now in nows:
        for preset in ranges.PRESETS:
            for country, key in (('at', None), ('generic', None), ('generic', 'Pacific/Kiritimati'),
                                 ('generic', 'Etc/GMT+12'), ('generic', 'America/Santiago')):
                add(f'{preset}-{now}-{country}-{key}', now=now, country=country, preset=preset.upper(),
                    timezone=key)
    for case in cases:
        profile = {'tax_country': case['country']}
        now = datetime.fromisoformat(case['now'].replace('Z', '+00:00'))
        try:
            resolved = ranges.resolve_export_range(profile, preset=case['preset'], first_day=case['first'],
                                                   last_day=case['last'], timezone_name=case['timezone'], now=now)
        except AppError as exc:
            case['expected'] = {'error': dict(code=exc.code, message=str(exc), hint=exc.hint)}
            continue
        except Exception as exc:  # noqa: BLE001 - surfaced as internal errors
            case['expected'] = {'error': dict(code='internal_error', message=str(exc), hint=None)}
            continue
        if resolved is None:
            case['expected'] = {'none': True}
            continue
        expected = {'start': int(resolved.start.timestamp()), 'end': int(resolved.end.timestamp())}
        try:
            expected['payload'] = resolved.payload()
        except Exception as exc:  # noqa: BLE001
            expected['payload_error'] = str(exc)
        case['expected'] = {'ok': expected}
    result['cases'] = cases
    return json.dumps(result, ensure_ascii=True, separators=(',', ':')) + '\n'


if __name__ == '__main__':
    check = '--check' in sys.argv
    content = build(copy=not check and '--keep-zones' not in sys.argv)
    if check:
        assert OUT.read_text() == content, 'range vectors drifted'
    else:
        OUT.write_text(content)

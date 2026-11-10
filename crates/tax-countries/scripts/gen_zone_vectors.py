#!/usr/bin/env python3
"""Write the UTC offsets Python's zoneinfo gives for the registry's zones.

`crates/tax-countries/src/calendar.rs` checks its rule-based zones against
them: a second before and at each summer-time change of a few years, and four
instants around midnight. Run with the host's tzdata:

    python3 -I crates/tax-countries/scripts/gen_zone_vectors.py > crates/tax-countries/tests/data/zoneinfo.txt
"""
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

ZONES = ["Europe/Berlin", "Europe/London", "Europe/Dublin", "Europe/Madrid", "Europe/Zurich", "Europe/Vienna", "Asia/Tokyo"]
YEARS = (1996, 2009, 2024, 2025, 2038, 2099)
MIDNIGHTS = ("2024-12-31T23:30:00+00:00", "2025-12-31T15:30:00+00:00", "2024-06-30T22:30:00+00:00", "1996-01-01T00:00:00+00:00")


def last_sunday(year: int, month: int) -> datetime:
    day = datetime(year, month, 31, tzinfo=timezone.utc)
    while day.weekday() != 6:
        day -= timedelta(days=1)
    return day


def main() -> None:
    with open("/usr/share/zoneinfo/tzdata.zi", encoding="utf-8") as file:
        print(file.readline().strip())
    for name in ZONES:
        zone = ZoneInfo(name)
        instants = []
        for year in YEARS:
            for month in (3, 10):
                change = last_sunday(year, month) + timedelta(hours=1)
                instants += [change - timedelta(seconds=1), change]
        instants += [datetime.fromisoformat(text) for text in MIDNIGHTS]
        for instant in instants:
            offset = int(instant.astimezone(zone).utcoffset().total_seconds())
            print(name, int(instant.timestamp()) * 1_000_000, offset)


if __name__ == "__main__":
    main()

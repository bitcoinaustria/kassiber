#!/usr/bin/env python3
"""Build Kassiber's transparent bundled BTC hourly rate CSVs.

The output keeps Kraken's seven-column OHLCVT CSV shape:

    timestamp,open,high,low,close,volume,trades

Rows from Kraken's own hourly archives are copied as-is. Before Kraken's BTC
markets have hourly rows, this script expands Kassiber's documented daily
Coin Metrics/ECB backfill into hourly fallback rows where
``open=high=low=close`` and ``volume=trades=0``.
"""

from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from pathlib import Path
import zipfile


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_KRAKEN_ROOT = Path("/Users/dev/Downloads/Kraken Data")
DAILY_DIR = ROOT / "kassiber" / "data" / "rates" / "kraken" / "btc_daily"
OUTPUT_DIR = ROOT / "kassiber" / "data" / "rates" / "kraken" / "btc_hourly"
INTERVAL_SECONDS = 60 * 60
DAY_SECONDS = 24 * INTERVAL_SECONDS
PAIRS = {
    "XBTEUR_60.csv": "XBTEUR_1440.csv",
    "XBTUSD_60.csv": "XBTUSD_1440.csv",
}


@dataclass(frozen=True)
class Row:
    timestamp: int
    open: str
    high: str
    low: str
    close: str
    volume: str
    trades: str

    @classmethod
    def from_csv(cls, cells: list[str]) -> "Row":
        if len(cells) != 7:
            raise ValueError(f"expected 7 columns, got {len(cells)}")
        return cls(int(cells[0]), *[cell.strip() for cell in cells[1:]])

    @classmethod
    def synthetic(cls, source_timestamp: int, price: str) -> "Row":
        return cls(source_timestamp, price, price, price, price, "0", "0")

    def to_csv(self) -> list[str]:
        return [
            str(self.timestamp),
            self.open,
            self.high,
            self.low,
            self.close,
            self.volume,
            self.trades,
        ]


def read_rows(handle, name: str) -> dict[int, Row]:
    rows: dict[int, Row] = {}
    for line_number, cells in enumerate(csv.reader(handle), start=1):
        if not cells:
            continue
        try:
            row = Row.from_csv(cells)
        except Exception as exc:
            raise RuntimeError(f"{name}:{line_number}: invalid OHLCVT row") from exc
        rows[row.timestamp] = row
    return rows


def read_file(path: Path) -> dict[int, Row]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return read_rows(handle, str(path))


def read_zip_member(archive_path: Path, member: str) -> dict[int, Row]:
    with zipfile.ZipFile(archive_path) as archive:
        with archive.open(member) as raw:
            text = (line.decode("utf-8") for line in raw)
            return read_rows(text, f"{archive_path.name}:{member}")


def kraken_sources(root: Path, member: str) -> list[tuple[str, dict[int, Row]]]:
    sources: list[tuple[str, dict[int, Row]]] = []
    pre_dir = root / "Pre Incremental Updates till 2015"
    pre_file = pre_dir / member
    if pre_file.exists():
        sources.append((str(pre_file), read_file(pre_file)))
    for archive_path in sorted(root.glob("Kraken_OHLCVT_Q*.zip")):
        with zipfile.ZipFile(archive_path) as archive:
            names = set(archive.namelist())
        if member in names:
            sources.append((archive_path.name, read_zip_member(archive_path, member)))
    return sources


def daily_prices(daily_file: Path) -> list[tuple[int, str]]:
    prices: list[tuple[int, str]] = []
    for row in read_file(daily_file).values():
        close_timestamp = row.timestamp + DAY_SECONDS
        prices.append((close_timestamp, row.close))
    return sorted(prices)


def add_daily_derived_prefix(rows: dict[int, Row], daily_file: Path) -> int:
    if not rows:
        return 0
    first_kraken_close = min(rows) + INTERVAL_SECONDS
    prices = daily_prices(daily_file)
    added = 0
    price_index = 0
    current_price: str | None = None
    first_daily_close = prices[0][0]
    close_timestamp = first_daily_close
    while close_timestamp < first_kraken_close:
        while price_index < len(prices) and prices[price_index][0] <= close_timestamp:
            current_price = prices[price_index][1]
            price_index += 1
        if current_price is not None:
            source_timestamp = close_timestamp - INTERVAL_SECONDS
            if source_timestamp not in rows:
                rows[source_timestamp] = Row.synthetic(source_timestamp, current_price)
                added += 1
        close_timestamp += INTERVAL_SECONDS
    return added


def write_rows(path: Path, rows: dict[int, Row]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        for timestamp in sorted(rows):
            writer.writerow(rows[timestamp].to_csv())


def build(root: Path, output_dir: Path) -> None:
    for hourly_name, daily_name in PAIRS.items():
        rows: dict[int, Row] = {}
        sources = kraken_sources(root, hourly_name)
        if not sources:
            raise RuntimeError(f"could not find Kraken source rows for {hourly_name}")
        for _, source_rows in sources:
            rows.update(source_rows)
        synthetic = add_daily_derived_prefix(rows, DAILY_DIR / daily_name)
        output_path = output_dir / hourly_name
        write_rows(output_path, rows)
        print(
            f"{hourly_name}: {len(rows)} rows "
            f"({synthetic} daily-derived, {len(rows) - synthetic} Kraken hourly) "
            f"from {len(sources)} Kraken sources -> {output_path}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--kraken-root",
        type=Path,
        default=DEFAULT_KRAKEN_ROOT,
        help="Directory containing Kraken quarterly zips and the extracted pre-2015 update folder.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=OUTPUT_DIR,
        help="Destination for XBTEUR_60.csv and XBTUSD_60.csv.",
    )
    args = parser.parse_args()
    build(args.kraken_root.expanduser(), args.output_dir)


if __name__ == "__main__":
    main()

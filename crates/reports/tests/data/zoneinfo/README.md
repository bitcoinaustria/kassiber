Copies of IANA tzdata 2026e TZif files (public domain), taken from a Linux
`/usr/share/zoneinfo`. `scripts/gen_range_vectors.py` freezes CPython's
zoneinfo offsets from exactly these files, so the vectors do not depend on
the host's tzdata version.

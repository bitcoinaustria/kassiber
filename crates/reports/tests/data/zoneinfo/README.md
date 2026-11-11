Copies of IANA tzdata 2026e TZif files (public domain), taken from a Linux
`/usr/share/zoneinfo`. The range vectors froze CPython's zoneinfo offsets
from exactly these files (with the Python generator removed under plan 22;
it survives at the `archive/python-backend` tag), so the vectors do not
depend on the host's tzdata version.

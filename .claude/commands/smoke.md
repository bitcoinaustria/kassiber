Run the CLI smoke checks only — faster than the full gate, covers the behavior pin.

The CLI smoke suite now runs as recorded sessions: replay the sessions the old
`tests/test_cli_smoke.py` recorded against this checkout's binary, then run the
binary's own black-box tests. The recordings are locked test data, so fetch
them first if the lock changed.

```bash
./scripts/fetch-test-data.sh
./scripts/replay-sessions.sh --only test_cli_smoke
cargo test --locked -p kassiber --test binary
```

`--only` selects sessions whose test id or file name contains the text. Every
session that prints different output or leaves different state is reported as
a failure.

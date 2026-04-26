# Daemon Reference

Kassiber's desktop shell talks to the Python core through a local JSONL
daemon. The daemon is started by the Tauri supervisor, reads one JSON object
per line from stdin, and writes one JSON envelope per line to stdout.

Start it directly for development:

```bash
python -m kassiber --data-root /tmp/kassiber-demo/data daemon
```

The Tauri supervisor starts the same command. In development it prefers the
repo-local `.venv/bin/python`, falls back to `python3`, and accepts
`KASSIBER_DAEMON_PYTHON=/path/to/python` as an override.

The first line is always a lifecycle envelope:

```json
{"kind":"daemon.ready","schema_version":1,"data":{"version":"0.21.0","supported_kinds":["status","daemon.shutdown"]}}
```

Requests carry a caller-chosen `request_id`, a `kind`, and optional `args`:

```json
{"request_id":"status-1","kind":"status"}
```

Responses use the normal machine envelope plus the same `request_id`:

```json
{"kind":"status","schema_version":1,"data":{},"request_id":"status-1"}
```

Errors use the standard error envelope shape and also echo `request_id` when
the request supplied one. `daemon.shutdown` asks the daemon to write a final
shutdown envelope and exit cleanly.

Only `status` is backed by real data in the first slice. UI snapshot kinds
return `daemon_unavailable` until typed contracts and read models land.

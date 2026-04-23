# Machine Output Reference

Every Kassiber command supports machine-readable output.

Global flags:

- `--format {table,plain,json,csv}`
- `--output <path>`
- `--machine` as a shortcut for JSON
- `--debug` to include debug details on errors

## Success envelope

Successful commands emit:

```json
{
  "kind": "reports.balance-history",
  "schema_version": 1,
  "data": []
}
```

The exact `kind` varies by command, but the outer envelope shape stays the same.

## Error envelope

Errors emit:

```json
{
  "kind": "error",
  "schema_version": 1,
  "error": {
    "code": "validation",
    "message": "Invalid start timestamp 'not-a-date'",
    "hint": "Use RFC3339 UTC like 2025-01-01T00:00:00Z",
    "details": null,
    "retryable": false,
    "debug": null
  }
}
```

`--debug` may include stack traces and other sensitive context. Do not paste debug output publicly without reviewing it first.

## Safe-To-Record Contract

Normal success envelopes now follow a narrow safe-to-record contract for
secret-bearing backend and wallet config values.

- `backends list/get/create/update` redact raw credential values such as
  `auth_header`, `token`, and `password`, and expose presence through
  `has_*` flags instead
- `wallets get/create/update` redact raw descriptor material inside the
  returned config while preserving state flags such as `descriptor`,
  `change_descriptor`, and `descriptor_state`
- backend URLs in machine output drop embedded credentials and query strings

This contract is intentionally narrow. Addresses, notes, file paths, backend
labels, and other operational metadata may still be sensitive, and `--debug`
output is explicitly outside this contract.

## Output modes

- `table` is for terminal reading
- `plain` is compact key-value style output
- `json` is the canonical machine-readable envelope
- `csv` writes tabular list results for spreadsheet workflows

Not every command produces CSV-friendly output, but list-style commands generally do.

## Stability

The machine envelope is one of Kassiber's key contracts and is pinned by `tests/test_cli_smoke.py`.

For automation:

- prefer `--machine`
- key off `kind` and `schema_version`
- treat human-readable terminal output as non-contractual

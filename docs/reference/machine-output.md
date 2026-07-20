# Machine Output Reference

Every Kassiber command supports machine-readable output.

Global flags:

- `--format {table,plain,json,csv}`
- `--output <path>`
- `--machine` as a shortcut for JSON
- `--debug` to include debug details on errors
- `--diagnostics-out <path|auto>` to write a public-safe diagnostics report on error

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

## Public Diagnostics

`kassiber diagnostics collect` emits a public bug-report artifact:

```json
{
  "kind": "diagnostics.collect",
  "schema_version": 1,
  "data": {
    "report": {
      "report_schema_version": 1,
      "public_safe": true,
      "environment": {},
      "invocation": {},
      "storage": {},
      "state": {},
      "checks": {}
    },
    "saved": null
  }
}
```

Use `kassiber --machine diagnostics collect --save` to also write the report
under `exports/diagnostics/` in the active state root. Use
`--diagnostics-out auto` before a failing subcommand to save the same public
report only when that command errors.

Diagnostics reports preserve shape, counts, and sanitized code context. They do
not include raw txids, addresses, descriptors, xpubs, labels, notes, exact
amounts, exact rates, backend hostnames, local paths, raw config, raw API
payloads, imported rows, or stack locals. `--debug` remains private.

Desktop Logs also provides support bundles for cases where a short issue
description plus correlated daemon and AI records is more useful than a CLI
diagnostics report. Support bundles are explicit exports from the Developer
tools-gated Logs page. They are `.support.jsonl` files with a manifest,
redaction report, recent redacted events, last-failure context, and redacted AI
provenance. High-signal mode is the default and keeps addresses, labels, paths,
URLs, and daemon error text readable for trusted maintainer debugging; amounts
and txids remain pseudonymized in both tiers. Public-safe mode masks the other
operational values for public posting. Both modes still exclude or redact the secret floor: raw
daemon arguments, raw AI prompts, descriptors beyond script shape/derivation
hints, private keys, recovery phrases, API keys, passwords, bearer tokens,
cookies, raw config, database files, imported rows, and stack locals.

## Safe-To-Record Contract

Normal success envelopes now follow a narrow safe-to-record contract for
secret-bearing backend and wallet config values.

- `backends list/get/create/update` emit an allowlisted safe backend view,
  suppress raw credential fields and unknown config keys, and expose
  credential presence through `has_*` flags instead
- `wallets get/create/update` emit an allowlisted safe wallet config view,
  suppress unknown config keys, and preserve state flags such as
  `descriptor`, `change_descriptor`, and `descriptor_state`
- backend URLs in machine output drop embedded credentials and query strings

This contract is intentionally narrow. Addresses, notes, file paths, backend
labels, and other operational metadata may still be sensitive, and `--debug`
output is explicitly outside this contract.

Broker operation status is also machine-readable. Accepted work reports an
opaque `operation_id` and `queued`, `running`, `completed`, `failed`,
`cancelled`, or `result_unknown`. A non-interactive command without an active
lease returns `interaction_required`; scoped brokered commands without their
declared `--workspace`/`--profile` flags return `operator_scope_required`.
Status exposes only the public project id, categorical authentication method,
cumulative grant, expiry policy, default book identifiers, and queue/worker
counts—not a database path, passphrase, native lookup identifier, or key.

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

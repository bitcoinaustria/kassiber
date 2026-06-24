# Logging and diagnostics — RAM-only by design

Kassiber's logs never touch disk on their own. A persistent log file under
the data root would be a forensic artifact of financial activity that
outlives every redaction choice made in the UI, so the entire observability
surface lives in bounded in-memory buffers and reaches disk only when the
user explicitly exports it.

The guiding principle:

> **Capture everything into RAM, strip wallet/credential material before it
> ever enters a buffer, and let the user decide — per export, per tier —
> what leaves the process.**

## The three buffers

| Buffer | Lives in | Bound | Survives |
| --- | --- | --- | --- |
| Daemon ring ([`kassiber/log_ring.py`](../../kassiber/log_ring.py)) | Python daemon process | 5,000 records / 4 MiB | webview reloads, UI restarts |
| Supervisor stderr tail + lifecycle ring ([`ui-tauri/src-tauri/src/supervisor.rs`](../../ui-tauri/src-tauri/src/supervisor.rs)) | Tauri (Rust) process | 16 KiB tail, 64 lifecycle records | daemon crashes and restarts |
| Webview ring ([`ui-tauri/src/lib/appLogs.ts`](../../ui-tauri/src/lib/appLogs.ts)) | Browser/Webview JS heap | 10,000 records / 4 MiB | nothing (it is the view) |

The webview ring is the merge point. The desktop app polls
`ui.logs.snapshot` (a pre-unlock daemon kind — it works while the database
is locked, which is exactly when you need it) and the
`daemon_lifecycle_snapshot` Tauri command, and folds both into the webview
ring with `daemon-*` / `super-*` record ids. The Logs page, exports, and
support bundles therefore see one merged stream: daemon request logs,
Python tracebacks, third-party library records, supervisor lifecycle
events (spawn/exit/kill with the dying daemon's redacted stderr tail),
webview transport records, console output, React errors, and unhandled
promise rejections.

A hard crash of the whole app loses the buffers. That is the accepted
price of RAM-only; the supervisor preserving the daemon's stderr tail
across daemon restarts covers the common case (the Python side died, the
shell survived). Note that "RAM-only" is a policy about what Kassiber
writes, not a physical guarantee — swap files and OS crash dumps can still
page buffer contents to disk. Key material is protected against that by
the insert-time secret floor below, not by pretending RAM is hermetic.

## Two-stage redaction

**Secret floor — applied at insert, in every buffer.** Seed phrases,
extended private/public keys, descriptors, API keys, bearer tokens,
passphrase assignments. These are scrubbed by
[`kassiber/redaction.py`](../../kassiber/redaction.py) (Python),
`redact_sensitive_text` (Rust), and `redactSecretFloorText` (TypeScript)
before a record is stored, so this material never exists in any buffer and
cannot leak through later code paths, exports, or memory dumps.

**Operational redaction — applied at render/export time.** Amounts,
addresses, txids, paths, URLs, labels. The Logs page renders redacted by
default with a time-bounded raw view; exports choose `high_signal`
(operational data readable, for the maintainer or a trusted debugging
session) or `public_safe` (operational data masked, for public bug
reports). Because operational redaction is a view concern, the raw-view
window and the two export tiers all work from the same captured records.

**Txids and amounts are the exception — pseudonymized in *both* tiers, never
raw.** They are the wallet fingerprint: a single txid or amount in a bundle
handed to an AI debugging session (the common workflow after a test sync
against a real wallet) ties the log back to a real wallet on a block
explorer. So instead of "readable in high_signal" they are always replaced
with a pseudonym — a txid becomes `txid#<fnv>` and an amount becomes
`amount#<salted-fnv>`. Txids stay deterministic across the Python/Rust/TS
boundary so transaction correlation survives. Amounts are salted per runtime so
low-entropy values like `2500 sats` cannot be recovered by enumerating likely
amounts against a public token; the same amount still correlates within the
same redaction runtime. `high_signal` additionally appends a coarse
order-of-magnitude bucket to amounts (`amount#a1b2 (~0.01 BTC)`) for
sat/msat-scale and fee-plausibility debugging; `public_safe` drops the
magnitude. Addresses, paths, URLs and labels stay readable in `high_signal` as
before. The only place a raw txid/amount can still reach disk is the explicitly
watermarked, confirm-gated raw export (`redacted: false`). Market *rates*
(`BTC/EUR 64000.12`) are public data, not the user's amount, and stay readable
in `high_signal`.

The pseudonymizers live in `redactSecretFloorText`'s sibling helpers in
[`appLogs.ts`](../../ui-tauri/src/lib/appLogs.ts) (`pseudoTxid` /
`pseudoAmount`) and in `redact_operational_text` /
`redact_operational_value`
([`kassiber/redaction.py`](../../kassiber/redaction.py)). The Python copy runs
inside `sanitize_traceback_text` (so ring tracebacks, `error.debug` and the CLI
`--debug` envelope are covered), over structured `error.details` at the daemon
error-envelope boundary (`redact_operational_value`, symmetric with the
secret-*key* scrub), and on the freshness disk write / UI snapshot — the
egresses that do not pass through the webview renderer (a backend exception's
`details` can carry a node `stderr` blob or `response_preview` with txids).

Prefer typed fields over free text when adding log producers: a field
typed `address`/`txid`/`path` is masked by type at render time, while free
text relies on the regex backstop, which is best-effort. Keyed/glued sat
amounts (`amount_sat=50000`, `fee_msat: 100000`, `"value_sats":123`) are caught
by an identifier-aware detector, but a *free-standing* unit-less integer
(e.g. `amount 12345678` with no adjacent or glued `sats`/`BTC`) cannot be
auto-detected — emit it as a typed `amount` field, not interpolated into a
message.

## What gets captured

- **Daemon requests** — every JSONL request logs start/finish/duration/
  outcome at debug level (errors at warning/error) under
  `kassiber.daemon.request`, with the `request_id` stamped via a
  contextvar that propagates into the sync worker pools. `ui.logs.snapshot`
  itself is exempt so polling does not feed the ring it reads.
- **Python `logging`** — the daemon installs a ring handler on the root
  logger at DEBUG, so module loggers (e.g. `kassiber.core.rates`) and
  third-party libraries land in the ring instead of an unconfigured
  stderr. The daemon never gains a stderr logging handler; the smoke suite
  pins `stderr == ""` at clean shutdown.
- **Background workers** — the freshness worker and AI chat worker stamp
  their own correlation ids; their failures are logged to the ring in
  addition to the envelopes they already emit.
- **Webview** — daemon transport round-trips (including `error.details`
  and `error.debug` excerpts), `window.onerror`, `unhandledrejection`,
  `console.error`/`console.warn` (with re-entrancy and duplicate-burst
  guards), and a root React error boundary.

## Tracebacks in error envelopes

`internal_error` envelopes carry a sanitized traceback in `error.debug`:
paths relativized (never absolute, never containing the home directory),
secret-floor scrubbed, length-capped. The CLI sanitizes the envelope copy
of `--debug` tracebacks the same way while keeping the raw traceback on
the local stderr.

The AI surface never sees `error.debug`. Error envelopes are not embedded
in provider-bound content today, and `redact_tool_arguments`
([`kassiber/ai/tools.py`](../../kassiber/ai/tools.py)) drops `debug` keys
outright as the standing belt-and-suspenders, mirroring the Tier-3
redaction pattern from [lightning-opsec.md](lightning-opsec.md). Log text
shown to or read by an AI assistant must be treated as untrusted data —
log messages can embed remotely influenced strings (server banners, error
bodies).

## What reaches disk, and when

Only user-initiated actions: the Logs page export (`.md`/`.log`/`.jsonl`,
watermarked when raw), the support bundle (redaction report + failure
context), and `kassiber diagnostics`. The one third-party exception is
rp2's own import-time file log, which packaged builds confine to a scratch
temp directory (see [`kassiber/core/engines/rp2.py`](../../kassiber/core/engines/rp2.py)).

An always-on log file is a rejected design. If session recording to disk
is ever added, it must be opt-in, visibly active, tier-redacted at write
time, and deleted on toggle-off by default.

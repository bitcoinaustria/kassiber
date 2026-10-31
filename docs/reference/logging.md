# Logging and diagnostics — RAM-only by design

Kassiber's logs never touch disk on their own. A persistent log file under
the data root would be a forensic artifact of financial activity that
outlives every redaction choice made in the UI, so the entire observability
surface lives in bounded in-memory buffers and reaches disk only when the
user explicitly exports it.

The OpenCode 2 adapter installs null-device symlinks for the CLI's automatic
log files before launching it, including version checks, and verifies the sink
again before sending credentials or prompts. Its stdout/stderr diagnostics are
drained without retention. Stable POSIX builds are supported; Windows and
unaudited channel builds fail closed because the same discard guarantee is
not available. Native provider session storage is a separate boundary described
in [AI](ai.md#provider-setup); ordinary OpenCode sessions still persist there.

The guiding principle:

> **Capture everything into RAM, strip wallet/credential material before it
> ever enters a buffer, and let the user decide — per export, per tier —
> what leaves the process.**

## The process-local buffers

| Buffer | Lives in | Bound | Survives |
| --- | --- | --- | --- |
| Daemon ring ([`kassiber/log_ring.py`](../../kassiber/log_ring.py)) | Python daemon process | 5,000 records / 4 MiB | webview reloads, UI restarts |
| Operator ring ([`kassiber/operator/service.py`](../../kassiber/operator/service.py)) | Per-login-user broker process | 5,000 records / 4 MiB | individual CLI/agent exits only |
| Supervisor stderr tail + lifecycle ring ([`ui-tauri/src-tauri/src/supervisor.rs`](../../ui-tauri/src-tauri/src/supervisor.rs)) | Tauri (Rust) process | 16 KiB tail, 64 lifecycle records | daemon crashes and restarts |
| Webview ring ([`ui-tauri/src/lib/appLogs.ts`](../../ui-tauri/src/lib/appLogs.ts)) | Browser/Webview JS heap | 10,000 records / 4 MiB | nothing (it is the view) |

The desktop webview ring is the merge point for desktop-owned buffers. The desktop app polls
`ui.logs.snapshot` (a pre-unlock daemon kind — it works while the database
is locked, which is exactly when you need it) and the
`daemon_lifecycle_snapshot` Tauri command, and folds both into the webview
ring with `daemon-*` / `super-*` record ids. The Logs page, exports, and
support bundles therefore see one merged stream: daemon request logs,
Python tracebacks, third-party library records, supervisor lifecycle
events (spawn/exit/kill with the dying daemon's redacted stderr tail),
webview transport records, console output, React errors, and unhandled
promise rejections.

The operator ring is intentionally not merged into the desktop webview. It is
process-local security telemetry for the terminal broker, disappears with that
broker, and has no durable or desktop export path.

A hard crash of the whole app loses the buffers. That is the accepted
price of RAM-only; the supervisor preserving the daemon's stderr tail
across daemon restarts covers the common case (the Python side died, the
shell survived). Note that "RAM-only" is a policy about what Kassiber
writes, not a physical guarantee — swap files and OS crash dumps can still
page buffer contents to disk. Key material is protected against that by
the insert-time secret floor below, not by pretending RAM is hermetic.

## Egress auditor

The desktop Egress screen is backed by a separate RAM-only ring in
[`kassiber/egress_ledger.py`](../../kassiber/egress_ledger.py), exposed via
`ui.egress.snapshot`. It records outbound metadata only: subsystem, host,
port, operation, method, proxy flag, and outbound byte count. It deliberately
drops paths, query strings, request bodies, headers, tokens, descriptors, and
AI prompts before insert.

`ui.egress.snapshot` is pre-unlock-safe like `ui.logs.snapshot`; before the
database is unlocked it returns captured rows and the SQLCipher header proof
while marking allowlist matching incomplete. After unlock, rows are annotated
against configured backend hosts, configured AI provider hosts, and built-in
price-provider hosts. The desktop table rows are expandable, but the expanded
view still shows only the stored metadata and a copy of the stored record. It
does not reconstruct raw paths, query values, bodies, headers, credentials,
descriptors, or AI prompts because those fields were discarded before the RAM
buffer. The `update` subsystem is reported in the summary and should remain
zero unless an explicit updater is added later. The auditor never opens sockets
itself, so it must not become a health checker or background probe.

## Two-stage redaction

**Secret floor — applied at insert, in every buffer.** Seed phrases,
extended private/public keys, descriptors, Silent Payments `sp()` / `spscan` /
`spspend` material, BDK/LWK observer blobs or opaque store values, API keys,
bearer tokens, and passphrase assignments. These are scrubbed by
[`kassiber/redaction.py`](../../kassiber/redaction.py) (Python),
`redact_sensitive_text` (Rust), and `redactSecretFloorText` (TypeScript)
before a record is stored, so this material never exists in any buffer and
cannot leak through later code paths, exports, or memory dumps.

The Python and webview floors also scrub `rpcpassword`, `rpcauth`, `password`,
`macaroon`, `cookie`, `secret_key` and `private_key` assignments. Their shared
scanners handle key-based secrets before URL userinfo, including escaped and
truncated diagnostics. Exact examples live in
`tests/fixtures/secret_redaction_cases.json`; the shared seeded generator adds
100,000 compositions with secret-fragment and non-secret preservation checks.
See [the scanner design and limits](#free-text-credential-scanner-and-limits)
for encoding support, ambiguous URL text and prose-safe flag handling.

The native Rust floor still uses its legacy token scanner. It must adopt the
same scanner, aliases, exact corpus and generator during Rust integration;
native-only stderr does not yet have Python/webview parity.

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

Complete UUID-shaped tokens (8-4-4-4-12 hex digits) and 32-hex IDs are
excluded from both free-text amount passes, including keyed sat values and
currency-symbol matches. Matching is case-insensitive and requires token
boundaries (no adjacent ASCII letters, digits, underscores or hyphens). Partial
UUID lookalikes are not exempt. The txid pass still runs first and is unchanged.
Only the ID's own characters are shielded: a comma or decimal point followed
by a separate amount is still redacted even when the ID ends entirely in
digits. Python and webview regression tests share exact output vectors and a
fixed test salt; runtime amount salts remain private and random.

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
- **Handled freshness failures** — a failed/deferred job logs its categorical
  job type, last recognized processing phase, allowlisted error code, and known
  exception class, with the request correlation id. Known SQLite error names
  and observer-conflict kinds provide additional discriminators. These records
  omit source labels, wallet identities, exception messages, and arbitrary
  exception details. Unrecognized codes use `freshness_job_failed` in the log;
  the scoped job result retains its original code for UI inspection. Cached batch
  failures use `batch_prefetch`: shared batch progress cannot establish which
  wallet was in its last reported decode phase.
  A completed refresh request also logs categorical job counts, distinguishing
  selected, attempted, deferred, cancelled and failed jobs without result blobs.
- **Operator broker** — unlock, lock, expiry, rejection, queue admission,
  dispatch, cancellation, and crash/result-unknown state enter only the
  broker's bounded Python ring. Fields contain opaque project ids, exact
  capability/command codes, and categorical auth methods; they omit raw argv,
  paths, endpoints, credentials, and secret frames. The ring dies with the
  broker and has no generic export or durable activity table.
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
context), and `kassiber diagnostics`. Packaged builds do not include RP2,
and no product module imports it (`tests/test_tax_oracle_imports.py`). Only
tests load it, under the tax engine's frozen parity oracle, which then removes
RP2's import-time file log from a scratch temp directory (see
[`tests/tax_oracle/adapter.py`](../../tests/tax_oracle/adapter.py)).

An always-on log file is a rejected design. If session recording to disk
is ever added, it must be opt-in, visibly active, tier-redacted at write
time, and deleted on toggle-off by default.

## Free-text credential scanner and limits

Python and the shipped webview run the same forward scanners before the remaining
key/descriptor backstops. Key assignments, JSON keys, authorization headers and
command flags run first. URL scanning therefore cannot swallow a later secret
key and expose the part of its value after `@`. Identifier tokenization and a
finite alias table cover prefixed, dotted, dashed, underscored and versioned
keys, including `secret_key` and `private_key`. Colon-separated URL userinfo is
left for the URL scanner, even when its username is itself a secret-key alias.
Unquoted assignments recognize `&` and `|` record separators.

Quoted values use escape-run counting rather than a backtracking expression.
The supported envelope is ordinary JSON, up to three enclosing JSON string
layers, and their Python `repr()` forms. Complete quoted values may span lines;
truncated values are masked through their line, and ambiguous glued quote tails
fail closed through the line. Deeper or arbitrary encodings are best-effort.
Do not emit raw secret-bearing argv, payloads or configuration and rely on this
scanner to recover their schema.

Space-separated flags never consume a following option beginning with `-`.
Outside recognized command lines they also leave common help words and tokens
ending in sentence punctuation readable. Command lines beginning with `cmd`,
`kassiber`, `bitcoind`, `bitcoin-cli`, `electrs`, `lncli`, `lightning-cli`, `curl`,
`python`, `node`, `env`, `sudo` or `docker` establish value context, including a
leading `$` or `>` prompt. `--seed-words` followed by a standard mnemonic word
count remains help/count metadata. A bare value that collides with help prose
is inherently ambiguous; producers must use a typed secret field, an explicit
assignment, or a quoted value. Colon status literals `none`, `expired` and
`(not set)` remain readable; `=` assignments are always secret-bearing.

The URL scanner recognizes HTTP(S), TCP, SSL, SOCKS5/H, WS(S), FTP, Postgres and
Electrum schemes. A userinfo candidate is bounded by whitespace, quotes, angle
brackets, square brackets and `/`. Commas, semicolons, colons, `#`, `?`, backslashes
and percent escapes are supported within userinfo. It removes through the last
`@`, covering raw `@` inside passwords. Malformed candidates crossing an in-token
record delimiter are masked through the remaining token rather than preserving
a possible password tail as a hostname. Numeric ports are never used to rule out
userinfo: ambiguous `http://h:8080#frag@x` is masked. Credential-free IPv6 hosts
and paths remain readable. A raw `@` in a query-like token (for example
`http://h?email=a@b`) is indistinguishable from the permitted `?`/`=` userinfo
characters and is masked too; structured URL producers should percent-encode
query values. Untyped URL text can be ambiguous, so this is a disclosure
backstop, not URL validation.

All scanner cursors move forward, failed candidates are skipped as a unit, and
quote decoding uses a fixed set of escape layers. The seeded shared generator
runs 100,000 compositions in Python and through the webview insert path. Both
assert that no four-character planted secret fragment survives, that planted
identifiers/prose/help remain readable, and that the complete output streams
have the same SHA-256 digest. The existing webview bare-mnemonic heuristic
retains its three documented corpus differences from Python. The native Rust
secret floor and supervisor need the same scanner and generator before they can
claim this free-text contract; forwarding already-redacted Python records does
not cover arbitrary native stderr.

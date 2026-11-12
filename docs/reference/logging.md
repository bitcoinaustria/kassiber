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
`tests/fixtures/secret_redaction_cases.json`; six seeded generators
each add 100,000 compositions with secret-fragment and non-secret preservation
checks. Nostr `nsec` private keys also pass through the key-material backstop.
See [the scanner design and limits](#free-text-credential-scanner-and-limits)
for encoding support, ambiguous URL text and prose-safe flag handling.

The native Rust floor still uses its legacy token scanner. It must adopt the
same scanners, aliases, exact corpora and all six generators during Rust
integration; native-only stderr does not yet have Python/webview parity.

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

Redaction fails closed: when a credential's extent is ambiguous, mask more.
Losing a following flag name, query parameter, hostname or malformed record tail
is acceptable; exposing part of a credential is not. Ordinary prose words are
the exception, so help text remains readable. Producers must use typed secret
fields rather than rely on inference from arbitrary text.

Python and the shipped webview run mirrored forward scanners before the remaining
key/descriptor backstops. Each scanner (URL userinfo, schemeless userinfo, XML,
name/value pairs, command flags and key assignments) reports spans in the
original text, over both the raw text and the decoded view. Their union is
masked; overlapping spans merge into one `[redacted]`. One detector therefore
cannot erase the context another needs, and a URL scan cannot swallow a later
secret key and expose the part of its value after `@`.
The keyed pass also retains URL credential context across malformed slashes and
whitespace, so masking a keyed value cannot erase the terminator and expose its
surrounding username. Complete endpoints and clear quoted record boundaries with
literal delimiters end that context; numeric URL ports remain readable. Encoded
quotes and record-like text inside malformed userinfo cannot establish a boundary.
Secret names are matched word by word, never as raw substrings. Identifiers
split on separators, camelCase and digit-to-letter boundaries. A concatenated
part that segments entirely into known secret and qualifier words is read as
those words (`rpcpassword`, `PGPASSWORD`, `dbprivkeyraw`, `masterseedwords`,
`BTCPAYPWD`). A part that does not segment fails closed: it is secret when it
contains a secret word or compound outside a listed ordinary word, whatever
unknown prefix or suffix surrounds it (`krakenpassword`, `btcpass`, `hdseed`,
`nodewif`, `lnrune`, `cardpin`, `smsotp`, `passwordbackup`, `PASSWORDconfirm`,
`tokenstring`, `macaroonhexstring`, `seedbackup`, `rpcpasswordish`). The
ordinary list is explicit: `bypass`, `compass`, `encompass`, `surpass`,
`trespass`, `overpass`, `underpass`, `passable`, `passage`, `passed`,
`passenger`, `passerby`, `passing`, `passive`, `passover`, `passport`,
`passthrough`, `passwordless`, `dnsseed`, `seednode`, `seeded`, `seeder`,
`prune`, `spin`, `swift`, `wifi`, and the stems `tokeniz` (`tokenizer`,
`tokenized`) and `secretar` (`secretary`). An ordinary word counts only at the
start of the part or after known qualifier words (`mytokenizer`), so
`ftprune` (`ftp` + `rune`) and `autoprune` are secret. Case runs are hints:
`rpcSECRETkeyOLD` and `myPASSWORDbackup` are also read concatenated. So
`prune`, `pruneheight`, `dnsseed`, `seednode`, `seeders`, `passed`,
`bypass_cache`, `passive`, `wifi`, `tokenizer`, `spinner` and `sighash` stay
ordinary vocabulary. Adjacent words form the explicit compounds
`access key`, `api key`, `auth key`, `hmac key`, `private key`, `secret key`,
`subscription key`, `seed words`, `recovery phrase`, `recovery code`,
`backup code`, `session id`, `pin code`, `otp code`, `totp code`, `mfa code`,
`2fa code`, `verification code` and the Silent Payments names, with their
plurals (`backup_codes`, `recoverycodes`). `passwd`, `pwd`, `passcode`,
`credential`, `bearer`, `jwt`, `session`, `sessid` (`PHPSESSID`), `sid`,
`sig`, `sign` (`CB-ACCESS-SIGN`, `API-Sign`, `X-BAPI-SIGN`), `signature`
(`X-Amz-Signature`), `mfa`, `twofa`/`2fa` and a `?key=`/`&key=` query
parameter are aliases. `user`, `auth`, `jwt`, `session`, `sid`, `sig`, `sign`
and `signature` must be a whole identifier part, and `pw`, `pin`, `otp`,
`totp`, `mfa`, `twofa` and `sign` the final word, optionally followed only by
the modifiers `old`, `new`, `backup`, `confirm`, `repeat`, `copy`, `value`,
`hex`, `raw`, `enc`, `encoded` and `plain` (`db_pw`, `pwold`, `pin_new`; not
`spinner`, `pw_hash`, `mfa_method` or `X-BAPI-SIGN-TYPE`). A spaced name such
as `api key:`, `PIN code:` or `"Secret Key":` is secret when the second word
changes the first word's meaning; `user name:`, `token count:` and
`see details:` are not.
Metadata suffixes `-file`, `-path`, `-fd`, `-stdin`, `_hint`, `_statistics`,
`_enabled` and `_disabled` do not become inline secret values, including in
concatenated forms such as `rpccookiefile` and `macaroonpath`.

An integer, decimal or boolean after a count-shaped name (a plural such as
`max_tokens`, `descriptors` or the compound `disable_private_keys`, or a
`count`, `max`, `min`, `limit`, `length`, `len`, `size` or `ttl` word) is
metadata and stays readable when it is the whole value: only whitespace
follows through the end of its line and any continuation, closers and a
record boundary (`,`/`;` before a quoted key, whitespace or prose), or, in a
command flag or a recognized command line, whitespace before a known flag or
a `name=` argument. So `bitcoin-cli -named createwallet wallet_name=cold
descriptors=true disable_private_keys=true blank=true` stays readable, while
`tokens=5 remaining`, `descriptors=true Pw9Leak` and `token_ttl=3600 Pw9Leak`
are masked; `num` is not a count word and `password=true` is a password.
A JSON literal closed by `}`, `]` or `)` ends at that closer even when an
indented line follows (`{"pin": 7368}` then `  next=1` keeps `next=1`); a bare
literal still owns its indented continuation. After a quoted JSON or `repr()` key,
`null`, booleans (`None`, `True`, `False`), sampling `seed` numbers, numeric
counts and a flat object of numeric members (`"prompt_tokens_details":
{"cached_tokens": 0}`) stay readable, so redacted provider error bodies still
parse; any other JSON literal is masked on its own (`{"pin":[redacted],…}`).
A string value never stays readable. Assignments accept `=`, `:`, `=>`, `:=`, `->` and repeated/spaced
`=` operators. Unquoted `&` and `|` remain part of an ambiguous value, including
query tails. Cookie/cookies assignments, objects, headers and curl `-b` lose all
cookie pairs; authorization and cookie headers include indented continuations.
The short `-u` and `-b` forms also work without a command prefix. A bare flag
can consume a value on the next line; trailing flags in help prose cannot.

The short aliases `pass`, `user` and `auth` are credential-bearing after a glued
colon, in assignments and command flags, and in prefixed/list/comment contexts.
Only a spaced colon followed by recognized ordinary user/status prose is exempt
(for example `user: alice logged in`, `auth: ok`, or `tests pass: 12`). A lowercase
password such as `pass: correcthorse` remains secret. The XML equivalents
`<user>alice</user>` and `<auth>required</auth>` require the whole value to match
the prose exception. Every exception applies only when it is the entire value,
including indented continuation lines: `user: alice Pw9Leak`,
`password: none Pw9Leak`, `--password the Pw9Leak` and `--seed-words 24 Pw9Leak`
are masked. `credentials` remains a secret alias. Colon status literals `none`,
`expired` and `(not set)` remain readable on their own; `=` assignments are
secret-bearing.

An unquoted value after a secret key is masked through the end of its line and
any following lines indented deeper than that line's own indent (past YAML
`- ` markers), so multi-word passphrases, shell `\` continuations and a stray
quote inside the value (`password=x" Pw9Leak`) cannot expose later words. Every
later secret key on the masked line still establishes its own block, quoted
value or continuation (`rpcpassword=x macaroon: |`). A value ends earlier only
when its end is unambiguous: a closing quote, a JSON literal after a quoted
key, or, for a command flag or a key inside a recognized command line,
whitespace followed by a known flag: a long `--flag`, a `-name=` assignment, a
letter flag before a `NAME=` argument (`docker run -e NETWORK=mainnet`), `-h`
or one of Bitcoin Core's argument-free options such as `-named`, `-txindex`,
`-regtest` or `-daemon`, optionally negated with `-no` (`-rpcpassword=x
-prune=550` keeps `-prune=550`). Any other word beginning with `-` is
passphrase data: `--passphrase correct -horse battery` and
`cmd --token Pw9 -v Leak` mask through the line. Losing the rest of a line
otherwise, such as a following `other=public` pair or a positional argument,
is the deliberate cost.

XML scanning covers namespaces, opening-tag attributes, key/value attributes
such as `<add key="Password" value="…"/>`, unquoted attributes,
`<input type=password value=…>`, `<meta name="csrf-token" content="…">`, plist
`<key>password</key><string>…</string>` pairs and named parameter contents.
Name/value records such as Kubernetes `name: X_PASSWORD` / `value: …` (separated
by any line break, including U+0085 and U+2028) and JSON
`{"name":"password","value":"…"}` mask their value. Tag and plist text is
trimmed of space, tab, CR, LF, VT and FF only, identically in both runtimes. Only a
complete matching closing tag ends an element value; a longer tag name sharing
the same prefix cannot expose the remaining value. A secret key/name attribute
also overrides an outer element's ordinary alias prose exception.
Structured `{…}`, `[…]`, `(…)`, `$(…)` and `${…}` values consume their
contents, including ambiguous glued tails. Unclosed openers (`"`, `'`, `"""`,
`(`, backtick, `<`) fail closed through their indented continuation. YAML `|`/`>-` blocks, triple quotes and adjacent quoted values are masked
through the extended value rather than only the first token.

Quoted values use escape-run counting rather than a backtracking expression.
Ordinary JSON, enclosing JSON string layers and their Python `repr()` forms do
not have a fixed nesting cap: delimiter runs are counted directly, without
recursive JSON decoding. Complete quoted values may span lines; truncated values
are masked through their line, and ambiguous glued quote tails fail closed
through the line. A comma followed by a quoted key must include the closing
key delimiter and assignment separator to establish a record boundary; glued
text after closing brackets/braces remains part of the ambiguous value.
This is a credential syntax scanner, not an arbitrary data format parser.
Arbitrary encodings, unknown aliases and unmarked prose values remain outside
its contract.

Secret-named space-separated flags consume their next token even when it begins
with `-`, so `--password --verbose` masks `--verbose`. Outside recognized command
lines, common help words and lowercase prose words ending in sentence punctuation
remain readable. Credential-shaped tokens do not become prose merely because
they end in punctuation. Command lines beginning with `cmd`, `kassiber`,
`bitcoind`, `bitcoin-cli`, `electrs`, `lncli`, `lightning-cli`, `curl`, `python`,
`node`, `env`, `sudo` or `docker` establish value context, including a leading `$`
or `>` prompt. `--seed-words` followed by a standard mnemonic word count remains
help/count metadata. Both runtimes treat CR, LF, U+0085, U+2028 and U+2029 as
line starts and use ASCII identifier/scheme boundaries, including next to
non-ASCII prose.

The `://` and `//` delimiters establish URL context whatever precedes them, so a
lookalike, zero-width or punctuation character in the scheme (`httpѕ://`,
`http\u200b://`, `http_://`) cannot exempt userinfo, and quoted or glued
scheme-relative `"//user:password@host"` is covered. The scanning view
normalizes fullwidth ASCII, the long-s and small-commercial-at lookalikes.
Schemeless `token:token@host` userinfo is masked with or without a key
(`https_proxy=`, `DATABASE_URL=`, `dsn=`), including before a later scheme in
the same token; `mailto:`, `xmpp:` and `sip:` addresses remain readable. Every scheme occurrence gets a bounded candidate, including URLs
inside paths, query values, archives and comma/semicolon/pipe lists. Ambiguous
prefixes survive nested-scheme
detection so a credential cannot hide by containing `://` itself.

All RFC 3986 userinfo characters, percent escapes and repeated `@` are covered;
masking reaches the last ambiguous userinfo terminator. A nested delimiter glued
to an unfinished authority (`a%40b:#x%2F%2F@host`) continues that userinfo
rather than starting a new URL. A decoded space or line
break remains credential data: only whitespace present literally in the original
text can establish a token boundary. Malformed or ambiguous credentials can
extend across literal slashes, whitespace and line breaks and may remove host
context. This is deliberate: losing context is preferable to leaking a byte.

Readability exceptions apply only to clearly credential-free URLs: ordinary
paths, handles and package/version references (such as `medium.com/@bob/post`,
`@scope/name` and `pkg@1.2.3`), and a following email on another line, without a
user:password shape or opaque credential-looking token. A recognized ordinary
host/path before `@` cannot acquire userinfo from that later `@`. Opaque runs,
malformed authority prefixes, ambiguous ports and credential-shaped path text
remain conservatively masked. For example, `HTTP://~/opaqueCredential@host`
is not treated as an ordinary public path. A complete credential endpoint also
ends at original whitespace, preserving an unrelated following line; an encoded
`@` ends it only before a dotted, bracketed or port host or ordinary words, since
`%40q (TOKEN@host` may be one malformed userinfo. A line break after an encoded
`@` never ends it when the next line's first token reaches an `@`
(`https://admin:Pw9%40Le` wrapped before `ak@db.example.org`): the wrapped line
is still password. A literal `#` after a bare token that is not a dotted, port
or bracketed host starts no fragment when a later `@` follows
(`https://Pw9#Leak@host` is userinfo through the `@`), while
`https://example.com/#/user@host` stays readable. A dotted user with a numeric
password (`https://alice.smith:12/34@host`) is userinfo, not a host and port.
A complete public URL followed by literal whitespace and a later email, handle
or path (`https://example.com/documentation please email a@b.c`) stays readable,
however long its path words, unless those words or the text before the later
`@` contain capital runs or mixed letter/digit runs; named query values are left
to the keyed scanner. A public `@handle` path cannot license `handle:password`.

The offset-preserving view recognizes JSON escaped slash runs and Unicode
escapes, numeric HTML entities (and `&colon;`, `&sol;`, `&commat;`, `&quot;`,
`&apos;`, `&lt;`, `&gt;`, `&amp;`, `&equals;`, `&Tab;`, `&NewLine;`) and up to three
percent-encoding
layers, including fully encoded nested URLs and keyed parameters. Both raw and
decoded views undergo keyed detection. Output retains original spelling outside
masked spans. Deeper percent encoding and other arbitrary encodings remain
best-effort. Never emit raw secret-bearing argv, payloads or configuration on
the assumption that this backstop can recover their schema.

Telegram `/bot<id>:<token>` paths, Slack `hooks.slack.com/services/…/…/<token>`
paths, Discord `/api/webhooks/<id>/<token>` and versioned `/api/v10/webhooks/…`
paths and Slack `xox?-` tokens for any letter (`xoxb-`, `xoxe-`, `xoxc-`) and
`xapp-` app tokens are masked through the next literal whitespace. Slack
`workflows/` and `triggers/` paths, and a `services/` path without both IDs,
are masked from the path kind onward. Once a URL's path, query or fragment starts, a literal line
break ends it unless the next line's first token continues the userinfo with an
`@`; a non-public URL therefore cannot merge with a later email or credential
URL on another line.

Candidate segments and structured scopes advance once; suffixes are not repeatedly
rescanned. Each record's line table (indents, the next shallower line and YAML
block ends) is built once, so a continuation end is a lookup however many keys
and lines share it, including an indentation staircase; name/value, userinfo
and count checks scan forward without backtracking. Escape-run counting and the fixed percent-decoding
passes are linear in input length; each adversarial shape is checked on its own
to 400,000 characters. A record longer than 1,000,000 code points is replaced
whole by `[redacted-oversized]` before any scanner runs. Six seeded generators
each run 100,000 compositions in Python and through the webview insert path. The
second to sixth retain the reviewers' independently authored distributions and
Python RNG; the fourth to sixth also run further seeds. The sixth checks every
planted name, flag, URL, provider and chat-service secret, including the
concatenated names, split flags and chat paths it was written to find. The fifth checks
planted secrets and numeric PIN/OTP codes inside the contract; zero-width noise
inside a key, prose-only provider errors, bare `twofa` and count-shaped numeric
names are outside it. All six assert
that no four-character planted secret fragment survives and that complete output
streams have the same SHA-256 digest in both runtimes; the first four also assert
that planted identifiers, hostnames, prose and help remain readable. The webview bare-mnemonic heuristic retains its three
known corpus differences from Python.

The native Rust secret floor and supervisor need the same scanners, aliases,
exact corpora and all six generators before claiming this free-text contract.
Forwarding already-redacted Python records does not cover arbitrary native
stderr. See the implementation verification report for the required Rust mirror.

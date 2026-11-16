# Desktop app contributors

These rules supplement [the root guidance](../../AGENTS.md) for the Rune desktop app. The
[README](README.md) covers layout, running it on a disposable book, and the checks; the full
gate lives in [CONTRIBUTING.md](../../CONTRIBUTING.md#verification-and-review).

## Shared runtime

The app runs the bundled `kassiber` binary as its daemon and speaks its JSONL protocol. Keep
accounting, review plans, consent effects, and readiness decisions in the shared Rust core
([backend guidance](../AGENTS.md)); the app renders and asks. Read
[the daemon contract](../../docs/reference/daemon.md#desktop-invoke-contract) before changing
transport or adding a desktop-invoked kind.

- `src/kinds.rs` is the desktop's privilege boundary: a kind the app sends must be in
  `ALLOWED_DAEMON_KINDS`, and `crates/daemon/tests/desktop_boundaries.rs` checks the lists
  against the daemon. Desktop invocation and AI tool exposure are separate authority
  decisions; do not add a kind solely to silence a drift test.
- Bind review plans and confirmations to the active book and input version (`expected_scope`
  on book-bound mutations). Keep amounts as exact decimal strings or integer msat and revision
  timestamps lossless. Display the server's plan before explicit confirmation; recompute
  effects server-side rather than treating client state as authorization.
- Use configured backend options for connection setup, not command templates. BTCPay
  provenance mapped onto an existing settlement wallet must not create a second balance source.
- Keep financial secrets and paths inside their native picker or credential flow. The
  assistant receives staging tokens for attachments, never raw file-path authority.
- Graph handoffs use `ui.chain_analysis.ai_context`; never interpolate raw graph identities into
  an assistant prompt. Privacy Mirror uses the shared analysis result and cannot add an
  independent detector, graph, or score.
- No network code in the app: every request goes through the daemon. The network chokepoint
  (`crates/net/tests/chokepoint.rs`) scans this crate; each process launch is a reviewed entry.
- Logs stay in the bounded, redacted RAM ring. The secret floor is the backend's own source
  (`src/secret_floor.rs` compiles `crates/envelope/src/redaction/`); do not fork it.

## Interaction and localization

- Every user-facing string goes through `src/i18n.rs`. Update the English and Austrian German
  bundles in `assets/i18n/locales/` together, follow
  [the localization workflow](../../docs/reference/i18n.md) and
  [glossary](../../docs/reference/i18n-glossary.md), and migrate whole surfaces. The scene
  check (`src/checks.rs`) fails on untranslated or unlabeled elements. CLI/daemon output stays
  English.
- Treat consent-gated tool cards as updates keyed by `call_id`: the same call can appear before
  consent and again when it starts running. Progress status is a loading hint, not
  chain-of-thought content.
- Route and developer-mode guards must apply to menu, deep-link, notification, and back/forward
  entry points as well as visible navigation. A hidden button is not an access boundary.
- Report freshness and partial sync errors stay visible as readiness state. A display read must
  not start a wallet or backend sync.

## Preview and packaging

Run the app on a disposable copy of a book with scratch preference files (`scripts/run.sh`);
never point it at the real `~/.kassiber` or keychain. Preserve another session's running app
and demo resources.

The app supervises the bundled daemon in packaged builds. Preserve request-ID routing,
cancellation, streaming, and unsolicited event handling when changing `src/daemon.rs` or
`src/app.rs`. Read [release packaging](../../docs/reference/prerelease-binaries.md) for the
bundle layout and installed-app CLI forwarding; do not assume a source checkout or a separately
installed `kassiber` exists on the user's machine.

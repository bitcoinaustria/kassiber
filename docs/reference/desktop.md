# Desktop Reference

Kassiber's desktop shell uses Tauri 2 + React + TypeScript with the Python
core running as a long-lived sidecar daemon over JSONL. See
[../plan/01-stack-decision.md](../plan/01-stack-decision.md) for the stack
decision and [../plan/04-desktop-ui.md](../plan/04-desktop-ui.md) for the
implementation plan.

The desktop shell is a pre-alpha preview. It already uses real daemon-backed
paths for the main setup, review, report, export, assistant, and diagnostics
workflows, but the CLI is still the most complete and scriptable control
surface. See [../quickstart.md](../quickstart.md) for the end-to-end
workflows and [machine-output.md](machine-output.md) for the JSON envelope
contract the shell consumes through the daemon.

The desktop UI is bilingual — English and Austrian German (informal `du`) — via
i18next. The active language lives in the UI store's `lang` (the single source
of truth) and is switchable from Settings → Appearance or the header overflow
menu; first run defaults to English. The CLI and Python daemon stay English and
machine-deterministic (the UI translates their stable codes). For how
translations are organized and the workflow for keeping English/German in sync,
see [i18n.md](i18n.md); for the Austrian-German terminology (Bitcoin jargon kept
English, BMF tax wording, `du` register), see [i18n-glossary.md](i18n-glossary.md).

Current development modes:

- `pnpm dev` in `ui-tauri/` runs the browser dashboard against the
  loopback-only Vite daemon bridge by default. `pnpm dev:bridge` is the
  explicit form of the same mode. Use `pnpm dev:browser` for the regtest demo
  browser preview. In bridge mode, the
  Welcome screen can open existing local books through a dev-only loopback
  folder picker; the Vite bridge validates the selected Kassiber data root and
  restarts its Python daemon with `--data-root` before the normal unlock/profile
  picker flow continues in the browser.
- `pnpm tauri:dev` runs the Tauri shell, starts `python -m kassiber daemon`,
  and calls the Rust `daemon_invoke` boundary. The command allowlists the
  current UI data, export, and action kinds. Report exports write under the
  managed `exports/reports/` state directory, and the desktop shell exposes a
  narrow `open_exported_file` command that opens completed PDF/XLSX/CSV report
  files with the system default app. Transaction explorer links use a separate
  `open_external_url` command that only accepts absolute HTTP/HTTPS URLs with a
  host and no embedded credentials before handing them to the system default
  browser.
  The Welcome screen can also open existing local books: the native
  folder picker opens at `~/.kassiber`, accepts either the state root or the
  `data/` folder, restarts the sidecar daemon with that `--data-root`, and then
  lists local books grouped by books set. SQLCipher databases must be unlocked
  before book names can be read because the internal workspace/profile rows live
  inside the encrypted database. macOS uses the system folder picker, Windows
  uses the system folder dialog through PowerShell/.NET, and Linux desktops use
  `zenity`, `kdialog`, or `yad` when one of those pickers is installed.
  The supervisor uses `.venv/bin/python` when present, then `python3`, unless
  `KASSIBER_PYTHON` is set. `KASSIBER_REPO_ROOT` can point a dev shell at a
  different checkout.

Current prerelease desktop packages bundle a one-file `kassiber-cli-*`
sidecar built with PyInstaller. At runtime the supervisor prefers
`KASSIBER_PYTHON` when it is explicitly set, then the bundled sidecar from the
app resources, then the development Python fallback above. The same
`KASSIBER_PYTHON` override applies to installed-app CLI forwarding.

For a real installable `.app` built locally on Apple Silicon (without
Rosetta), use `./scripts/build-macos-arm64-app.sh`. The build is unsigned
and ad-hoc signed; first-launch Gatekeeper handling is documented in
[prerelease-binaries.md → Local Apple Silicon build](prerelease-binaries.md#local-apple-silicon-build).

The authenticated shell includes a Diagnostics screen with a redacted
daemon/transport activity log and downloadable exports. It is meant for
prerelease and development troubleshooting: request logs include argument keys,
not argument values, while terminal daemon errors keep their structured
message, hint, and redacted details when the daemon exposes them. Every logged
daemon invoke gets a client request id and matching `trace_id`, so support
exports can group start, stream, terminal, and failure records.

For user-shareable troubleshooting, Logs offers **Export → Support bundle**.
The bundle is a `.support.jsonl` file containing the user's short issue
description, a manifest, a redaction report, recent redacted events,
last-failure context, and redacted AI provenance records. High-signal mode is
the default for trusted maintainer debugging and keeps operational values such
as amounts, txids, addresses, labels, paths, URLs, and daemon error messages
readable. Public-safe mode masks those operational values for public posting.
Both modes apply the secret floor: descriptors are reduced to script
shape/derivation hints, xpubs are stable-hashed, and private keys, recovery
phrases, API keys, passwords, bearer tokens, cookies, raw daemon arguments,
raw AI prompts, imported rows, database files, and stack locals are excluded or
redacted.

Transaction detail includes a Transaction flow panel on the Details tab. The
panel uses `ui.transactions.graph` to draw a local, read-only flow view: valued
Bitcoin vin/vout become proportional input/output strands with a distinct fee
leg, reference-only or confidential records can show amountless public
references, and unsupported imports get an explicit empty state instead of a
guessed graph. The view is explanatory, not a source of new accounting truth;
ownership tags such as owned wallet, external recipient, change, transfer,
swap, Coinjoin, blocker, or quarantine come from the same transaction graph and
manual-pair semantics used by the journal pipeline. If a public backend lookup
is allowed, sanitized tx/prevtx graph references are cached in the local DB so
reopening the panel can reuse them without exposing backend endpoints or raw
lookup material to the UI; the cache stores normalized graph refs rather than
raw serialized transactions, and successful Bitcoin graph lookups remain
complete for the current transaction. Hidden-sensitive mode keeps amounts and
long references masked. Reviewed paired routes, including swaps and
manual/AI-consented Coinjoin links, can show the spent and received legs; the
desktop preloads both safe graph payloads once the route is known so switching
between legs is UI-only when the daemon data is already available.

The Connections detail page includes a read-only UTXOs table for chain-backed
wallet sources. Refreshing a descriptor/xpub/address wallet updates the local
output inventory, and the detail page shows current unspent transaction outputs with
outpoint, amount, confirmation state, receive/change branch/index when known,
address or safe label, and source freshness. It shows all rows returned by the
daemon payload, reports when that payload is capped, offers sorting by size,
chain date, confirmations, or outpoint, and can open the UTXO's transaction in a
configured/public explorer after the same privacy warning used by transaction
detail explorer links. The table is
inventory-only: there is no spend, PSBT, signing, broadcast, coin-selection, or
freeze action.
Unsupported file/BTCPay/Lightning-style sources show an unsupported state, and
Liquid sources show an unblind blocker unless Kassiber has descriptor material
that can unblind outputs locally.

Privacy Mirror is the dedicated desktop page for local privacy linkage. It
reads `ui.reports.privacy_mirror` and shows exposure summary, adversary cards,
wallet/transaction/UTXO rows, timeline, coverage, unknowns, evidence
drilldowns, and a PSBT/what-if panel. Wallet detail and transaction detail
include compact Privacy Mirror panels from the same redacted payload. The page
is local-only, read-only, advisory-only, shows degraded states instead of
standing reassurance badges, and uses the normal desktop daemon allowlist; it
does not sync, sign, broadcast, select coins, or mutate accounting data.

Settings -> AI providers displays each provider's API-key presence plus storage
location/state. Saving provider metadata does not include the raw key in the
create/update request; when the API-key field is filled, the form sends the
narrow `ai.providers.set_api_key` daemon kind and receives only redacted
metadata back. Connection tests use the stored provider key; a newly typed key
must be saved through `ai.providers.set_api_key` before testing. Settings can
move an existing provider key between `sqlcipher_inline` and the native store
with `ai.providers.move_api_key`. macOS production-signed builds may default to
Keychain; unsigned/ad-hoc/unknown macOS builds keep Keychain opt-in
experimental copy because rebuilds or app identity changes can prompt again.
Windows uses user-scope Credential Manager/DPAPI when available. Linux falls
back explicitly to `sqlcipher_inline` when no desktop secret service, D-Bus
session, or unlocked collection is available. There is no
plaintext fallback and no remember-unlock behavior.

Settings -> Desktop -> Terminal command can install a user-local `kassiber`
launcher without administrator privileges. It writes a small managed launcher
under the user's bin directory (for example `~/.local/bin/kassiber`) that
forwards to the installed desktop executable and its bundled CLI sidecar. On
macOS and Linux it adds one clearly marked block to the current shell profile
when the user bin directory is not already on PATH; removal deletes only that
managed block. Apps launched from a DMG or macOS App Translocation must first
be moved to Applications so the launcher cannot point at a transient mount.

Native packages own the command integration for stable upgrades: the Homebrew
cask links the app's bundled launcher, Linux `.deb` installs
`/usr/bin/kassiber`, and Windows MSI/NSIS installers expose the bundled `bin`
directory on PATH and remove only their own entry during uninstall. Settings
recognizes these package-managed commands instead of offering to overwrite
them. AppImage and direct-DMG installs retain the user-local fallback. None of
these paths starts Kassiber automatically; the GUI, daemon, and CLI run only
when invoked.

Windows installer scope is deliberate: MSI is the machine-wide/admin route and
owns its system PATH entry, while NSIS installs for the current user and owns
only that user's PATH entry. Linux desktop and CLI-only Debian packages conflict
and replace one another cleanly because both intentionally provide the
`/usr/bin/kassiber` command; the desktop package carries GTK/WebKit, while
`kassiber-cli` does not.

Package managers can link the bundled launcher at
`Kassiber.app/Contents/Resources/bin/kassiber` instead. The first target is a
project-owned Homebrew tap; see [Homebrew Cask](homebrew-cask.md).

The GUI executable also works as a CLI forwarder when launched directly with
`--cli ...`. Examples:

```bash
Kassiber.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

If the app executable is symlinked with the exact executable stem `kassiber`,
plain CLI args are also forwarded, but the Settings launcher is preferred
because it explicitly passes `--cli`:

```bash
ln -s /Applications/Kassiber.app/Contents/MacOS/kassiber-ui /usr/local/bin/kassiber
kassiber status
```

Use `--cli ...` for any other symlink or executable name.

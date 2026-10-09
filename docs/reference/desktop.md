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
  picker flow continues in the browser. Leaving an imported folder restarts the
  daemon on the book the preview started with: `KASSIBER_DEV_DATA_ROOT` when
  set, otherwise the default root.
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
  folder picker opens near the effective platform app-data root, accepts either
  a project folder or its `data/` folder, restarts the sidecar daemon with that
  `--data-root`, and then
  lists local books grouped by books set. SQLCipher databases must be unlocked
  before book names can be read because the internal workspace/profile rows live
  inside the encrypted database. macOS uses the system folder picker, Windows
  uses the system folder dialog through PowerShell/.NET, and Linux desktops use
  `zenity`, `kdialog`, or `yad` when one of those pickers is installed.
  The supervisor uses `.venv/bin/python` when present, then `python3`, unless
  `KASSIBER_PYTHON` is set. `KASSIBER_REPO_ROOT` can point a dev shell at a
  different checkout.

Prerelease desktop packages bundle a `kassiber-cli-*` sidecar built with
PyInstaller. macOS ships it one-*dir*, as
`binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin` beside its
`_internal/` payload; Linux and Windows ship the flat one-file build. The
macOS split is a cold-start fix, not a packaging preference: a one-file
sidecar unpacks ~170 MB into a fresh temp directory on every launch, and macOS
re-validates the code signature of every bundled dylib from scratch each time
because that cache is keyed by inode. That cost 6s of off-CPU work before the
daemon answered, on every cold start; a stable path inside the bundle brings it
to 0.19s. Linux and Windows have no equivalent per-inode cost.

Every packaged artifact is built against uv's own managed CPython — the
prerelease workflow pins `UV_PYTHON`/`UV_PYTHON_PREFERENCE`, and
`scripts/build-macos-arm64-app.sh` passes `--python`. Keeping both on the same
interpreter is what makes a locally verified build and the published one the
same shape; that divergence is why this cold-start bug never reproduced
locally. `actions/setup-python` ships a macOS *framework* CPython whose stdlib
extension modules are 58 separate `.so` files, where the managed build links
them into `libpython3.11.dylib`: 115 bundled dylibs against 57, each validated
separately, plus symlinks inside the one-dir tree that Tauri cannot copy.

At runtime the supervisor prefers `KASSIBER_PYTHON` when it is explicitly set,
then the bundled sidecar from the app resources (one-dir candidate first), then
the development Python fallback above. The same `KASSIBER_PYTHON` override
applies to installed-app CLI forwarding.

On macOS the window uses an overlay title bar, and the shell draws the row the
traffic lights sit in: one 40px title bar carrying the nav toggle, history
buttons, book breadcrumb, alpha chip, and shell actions, over a flush side nav
and an inset page panel. Empty stretches of that row move the window through
Tauri's `data-tauri-drag-region="deep"`, which skips buttons and links. A modal
dialog makes the page inert, so `WindowFrame` also turns a press on the still
visible title bar into a window drag instead of letting it dismiss the dialog.
Portalled full-window surfaces (dialogs, sheets, the expanded chart, the lock
screen) start below `--kb-window-top-inset` so the title bar stays reachable. In
full screen the traffic lights are hidden and both insets drop to zero.
Windows and Linux keep their native decorated frame; the same row is an
in-app toolbar there with no inset and no drag region. Screens outside the shell
(setup, loading, the error boundary) and the lock and import-restore screens use
the same outline: the title bar row on the chrome and an inset page panel.

The side nav remembers whether it was folded to its icon rail, and folds by
itself while the window is narrower than 1100px without changing that choice;
the profile stays on the rail either way. Back and forward also run from
Cmd/Ctrl+[ and ] or Alt+Left/Right. History stays put while typing, with a
dialog open, or while locked; on Windows and Linux the shell swallows Alt+arrows
then, because WebView2 would otherwise navigate by itself. Tooltips and menus
write shortcuts in the platform's notation (`⌘⇧A` on macOS, `Ctrl+Shift+A`
elsewhere).

Every modal shares the command palette's treatment: dialogs, sheets, the
palette, and the sync progress card use the same faint, unblurred backdrop and
frosted surface, and dialogs hang from one anchor below the title bar
(`--kb-dialog-top`, capped by `--kb-dialog-max-h`) instead of centring, so a
dialog whose content changes size grows downward rather than jumping.

Screens share one layout scale, defined as tokens in `globals.css`. Every page
renders in the `lib/screen-layout.ts` frame: one gutter (`--kb-page-gutter`)
and one gap between blocks (`--kb-page-gap`). Corners come in three surface
tiers, outermost largest: the window tier (`--kb-radius-window`) for the page
panel, dialogs, the palette and the lock and setup panels; the card tier
(`Card` or `.kb-surface`) for a panel on a page; the inset tier
(`.kb-surface-inset`) for a tile inside a card. Controls stay `rounded-md` and
chips `rounded-full`. The title bar names the page and is its `h1`, so a page
does not repeat its name or add an eyebrow line. A page shows a title only when
the title bar does not already say it, such as a settings section or a wallet's
name, and that title uses `pageTitleClassName`.
`src/styles/layoutScale.test.ts` fails on bare `rounded` (square, because
`--radius` is 0), arbitrary radii and shadows, page glass, and hand-rolled
`rounded-* border bg-card` cards outside a short allowlist.

The command palette (Cmd/Ctrl+K) opens as a launcher: the main pages and
everyday actions with their shortcuts, before anything is typed. Page shortcuts
come from the native menu, so the browser preview does not show them. The
palette, its actions, and the workflow shortcuts stay closed while the app is
locked.

The main window starts hidden and is sized before it is shown. The first launch
centres a frame in the work area of the display it opens on (82% by 88%, capped
at 1440×960 and floored at the configured minimum). After that the last frame
is restored: macOS uses AppKit's frame autosave, which keeps the frame in the
app's preferences and fits it onto the displays that are connected; Windows and
Linux use `tauri-plugin-window-state`, which writes `.window-state.json` to
Tauri's config directory. Tauri's macOS config directory is Kassiber's state
root, which must not gain files before the default-root migration runs, which
is why macOS does not use the plugin.

The native desktop shell also carries a deliberately minimal release notifier,
modeled on Sparrow Wallet's cadence and manual-download flow. Setup explicitly
asks whether Kassiber may contact GitHub; no scheduled check is eligible before
that setup choice is persisted. Ten seconds after launch, and then every 24
hours while open, the desktop asks the bundled CLI sidecar to perform the
check (`kassiber --format json update`) when **Settings → Privacy → Allow
GitHub update checks** is enabled. The CLI owns the single GitHub code path:
prerelease/dev builds read one listing page — GitHub returns it newest-first, so
the highest published semantic-version tag is on it — and compare that tag with
the packaged version, while stable `release` builds use GitHub's latest-stable
endpoint so a run of prereleases cannot hide a stable update. Desktop-triggered
checks therefore share the CLI's response cache instead of contacting GitHub
separately, and dev builds run the same path as release builds rather than a
stub that always reports "current". If a newer version exists, the version
label at the bottom-left of the sidebar changes to an underlined
`update available · vLatest` link to that GitHub release page. The macOS
Kassiber menu also includes **Check for Updates…** directly below About; while
permission is enabled, an explicit check reports whether Kassiber is current
and offers to open the release page when an update exists, and reports the
specific reason when the check fails rather than a generic retry message.
Clicking the sidebar version line runs that same check while permission is
enabled; without it the line stays an ordinary link to the repository.
While disabled, the menu action reports that checks are disabled without
spawning the sidecar. The switch lives in the existing Privacy panel; there is
no separate update settings panel.
Kassiber never selects an asset, downloads a file, installs an update, or
restarts itself; automatic failures stay silent so they cannot interfere with
startup. The native shell independently
reads the same owner-only `<state-root>/config/update-checks.json` consent used
by the packaged CLI before spawning anything, fails closed when it is absent,
malformed, or disabled, and rejects any release URL that leaves the official
repository. The renderer hydrates from that canonical native preference at
startup and never restores consent from its own local storage; every check
re-reads the file before using its result, so a revocation made from the CLI
while the desktop is running still suppresses the result and turns the toggle
off. Consent writes are atomic and take effect immediately. See
[Privacy & security](privacy-and-security.md) for the outbound-request disclosure.
The update announcement remains unsigned and relies on HTTPS plus control of
the Kassiber GitHub repository. Release builds include a versioned SHA-256
manifest, and signed releases attach its detached OpenSSH signature from the
pinned release key. The notifier does not verify downloads or claim that
downloaded bytes are authenticated. See
[release signing](release-signing.md).

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

Transaction detail shows a Transaction flow panel beside its tabs, above them
in narrow windows; the review checklist sits in the header's timeline bar. The
panel uses `ui.transactions.graph` to draw a local, read-only flow view: valued
Bitcoin vin/vout become proportional input/output strands with a distinct fee
leg, reference-only or confidential records can show amountless public
references, and unsupported imports get an explicit empty state instead of a
guessed graph. Liquid legs this book's wallets unblinded during sync keep their
values, including spent inputs and another owned wallet's legs of the same
transaction; conflicting observations stay confidential. Strand widths and
positions follow mempool's bowtie graph unchanged (`TransactionGraphGeometry.ts`
ports its calcTotalValue, initLines and linesFromWeights): on Bitcoin the total
is the outputs plus the fee; on Liquid, with unknown legs on both sides, unknown
legs are assumed as large as the average known leg of their side and the
explicit fee is a known output; without any total every leg gets an equal
share; the outer ends of both sides fill the same height; legs past 250 fold
into one "+N more" leg. Widths are drawing only. The graph is drawn in 3D,
inline and in the expanded dialog, as that bowtie in glass after the Bitcoin
Austria artwork lab: one ribbon per input and output, as thick as mempool's
strand, with a block for the coin at its outer end (the fee has none). The
ribbons meet in one slim glass collar, since a transaction spends its inputs
together; their order does not say which input paid which output. Frosted
ribbons carry no known amount, and hidden values give every leg an equal share
of the same shape. The legend names only the swatches the drawing uses. As on mempool, the inputs and outputs are listed below the
graph, inputs on the left. An input whose coin a row in this book
created, and an output this book later spent, open that row in place: the book's
own history can be walked back and forth like an explorer, without a request,
and the header's Back button returns along the coins followed. Pointing at a
leg in the drawing lights it, shows its card and marks its row in the list;
pointing at a row lights its leg, and clicking an input or output leg brings
its row into view, unfolding its column if needed.
The view loads three.js only when a graph is shown,
renders only on change, needs no network, and falls back to the flat bowtie
without WebGL. After a wallet sync returns, a bounded background pass fills
missing graph references through that wallet's backend. Its completion refreshes
local graph queries. The "Look up on-chain" action appears only where a lookup
could still add something: a row with no local graph, or a Bitcoin graph whose
spent outputs lack their amounts, mostly rows without a synced wallet or left
incomplete by the pass's limits or a backend failure; a local Liquid graph is
never replaced. Opening the panel never starts a network request.
Manual lookups use the wallet's own backend first and never a server
Kassiber merely ships as a default, so the panel follows whatever observes the
wallet and stays silent instead of reaching for third-party infrastructure; with
nothing configured it offers an explicit backend-setup action. The view is
explanatory, not a source of new accounting truth;
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
detail explorer links. Above the table, the wallet's coins of its main asset
show as glass blocks in the transaction graph's style, one per coin and as tall
as its amount relative to the wallet's largest, unconfirmed coins frosted;
hidden values give every block the same height in outpoint order. Without
WebGL only the table shows. The table is
inventory-only: there is no spend, PSBT, signing, broadcast, coin-selection, or
freeze action.
Unsupported file/BTCPay/Lightning-style sources show an unsupported state, and
Liquid sources show an unblind blocker unless Kassiber has descriptor material
that can unblind outputs locally.

Overview, Wallets, the book-set overview, and the Reports metric strip read the
daemon's `fiat.completeness` block (see [the daemon reference](daemon.md)) instead of
trusting cost basis as-is. While it is incomplete, the fiat portfolio card turns amber
and shows an Incomplete or Outdated badge instead of the "vs cost basis"
percentage. It names the BTC without cost basis and links to Quarantine, or to
Journals when stale. The holdings header and chart summary drop their unrealized
percentage. Chart points from the first gap on show cost basis, average cost, and
unrealized as "—" with an explanation, the average-cost line stops there, and
earlier points keep their values. When average cost is visible, a hatched
region marks the plot from the first incomplete day onward (the whole plot if
the start is unknown). A label pinned to its boundary line, and the chart
summary's amber note, open the same explanation: BTC balances stay exact, and
each cause (stale journals, quarantine, custody gaps, missing prices) links to
the page that resolves it.
Without a market rate, fiat values show "—"
instead of €0 or -100 %. Custody gaps and missing prices also show in the
readiness pill and Book readiness panel. Book-set rows get a per-book badge, and
the fiat total says how many books are incomplete. Reports marks cost basis,
gain/loss, and estimated tax as Provisional while journals are stale or
quarantines, custody gaps, or missing prices exist. Reports copy stays English
under the deferred reporting-language policy in [i18n.md](i18n.md).

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

The same Settings screen carries **External agents (MCP)**, off by default and
disabled while AI features are off. Turning it on records the choice through
`ui.agent_access.configure` and shows a `claude mcp add` command, plus a
copyable JSON config, pinned to the current book. The desktop also mirrors the
AI features switch into that preference, writing only when it changed, so
`kassiber mcp serve` refuses agents while AI is off; see [MCP](mcp.md).

Settings -> Desktop -> Developer tools carries the early-stage-features switch,
which is off by default and gates the surfaces that are not finished yet. With
it off, Network Monitor, Logs, and Settings -> Lightning disappear from the
side nav, the settings rail, and app search, while Activity, Custody Gaps, Exit
Calculator, and Privacy Mirror stay in the nav as greyed-out, inert rows. The
nav treatment is a signpost, not the barrier: every one of those routes carries
a guard that redirects to Overview, the native menu's Logs item is disabled, and
a native-menu or `kassiber://` intent for any of them lands on Settings ->
Developer tools with an explanatory notification instead of a silent bounce. The
choice is stored in the desktop's local UI state, so it survives updates;
`ui-tauri/src/components/kb/devMode.ts` holds the route lists.

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
project-owned Homebrew tap; see [Homebrew](homebrew.md). The tap also carries
a GUI-free `kassiber-cli` formula built from the CLI-only release archives.

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

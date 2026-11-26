# Desktop Reference

Kassiber's desktop app is a native Rust UI built with
[Rune](https://github.com/anomalyco/rune), in
[`crates/kassiber-desktop`](../../crates/kassiber-desktop/README.md). It runs
the Rust `kassiber` binary as a long-lived sidecar daemon over JSONL, like the
CLI's other front ends, so the desktop and the CLI share one accounting core.
The crate is its own Cargo workspace (the root workspace excludes it) and
reads its translation bundles and integration logos from `assets/` at the
repository root.

```bash
cargo build --release -p kassiber                       # the daemon, from the root
crates/kassiber-desktop/scripts/run.sh --data-root <a disposable book>/data
crates/kassiber-desktop/packaging/ci/rune-checks.sh --skip-scenes   # fmt, clippy, tests
crates/kassiber-desktop/packaging/macos/build-app.sh --kassiber-bin target/release/kassiber --unsigned --output <dir>
```

The crate's [README](../../crates/kassiber-desktop/README.md) covers headless
snapshots, scenes, fixtures, the parity harness and packaging.

The Tauri 2 + React shell it replaced was removed; commit
`1c8065d4b676675076a8ca593963e2d4a121b43b` is the last that contains it. The
[original stack decision](../plan/01-stack-decision.md) and
[desktop plan](../plan/04-desktop-ui.md) are kept as history.

The desktop shell is a pre-alpha preview. It already uses real daemon-backed
paths for the main setup, review, report, export, assistant, and diagnostics
workflows, but the CLI is still the most complete and scriptable control
surface. See [../quickstart.md](../quickstart.md) for the end-to-end
workflows and [machine-output.md](machine-output.md) for the JSON envelope
contract the shell consumes through the daemon.

The desktop UI is bilingual — English and Austrian German (informal `du`) —
through `src/i18n.rs`, a subset of i18next over the shared bundles. The active
language is the app preference `lang` (the single source of truth, kept in the
desktop preferences file) and is switchable from Settings → Appearance or the
setup screen's language select; first run defaults to English. The CLI and
daemon stay English and machine-deterministic (the UI translates their stable codes). For how
translations are organized and the workflow for keeping English/German in sync,
see [i18n.md](i18n.md); for the Austrian-German terminology (Bitcoin jargon kept
English, BMF tax wording, `du` register), see [i18n-glossary.md](i18n-glossary.md).

The shell assistant opens in the right panel, laid out after T3 Code's: it
docks beside the page, which keeps working at its width minus the panel's (at
least 480px), and carries a tab per open thing: the assistant, the panel's own
surfaces, and open transactions' details, which open as tabs instead of a
second sheet. Every tab shares one width, dragged at the panel's left edge,
so switching tabs resizes neither the panel nor the page and does not fold or
unfold the side nav. The surfaces are the review queue (the causes the journals hold
transactions for, transfers still to match, and journals waiting to be
processed, read with the same daemon calls as the Review screen and the
sidebar's Review badge), Changes (recent edit history as a diff, with undo, from
the Activity page's history read) and Logs (a live tail of the redacted
in-memory log). Changes and Logs open only while developer tools are on,
like their pages, and close when they are turned off. A surface reads
nothing until it is opened. A transaction a page opens is that page's
preview tab (in italics), replaced by the next one the page opens; pinning it
(the pin button or a double click) keeps it open in a tab of its own, as is
every transaction opened from a surface, so several stay open side by side.
Closing a detail returns to the tab it was opened from. A tab's icon turns
into its close mark on hover; a middle click or Delete closes it too, and
only that tab. `+` in the tab bar adds a surface, and a panel whose last tab
closed shows a launcher of them instead. Each launcher row answers to a
single key (A for the assistant, Q for the review queue, H for Changes, L for
Logs, D for the latest detail) while no field, dialog or menu has the
keyboard; a row that cannot open says why. The title-bar toggle shows and
hides the panel and keeps all its tabs, including transaction details and
their unsaved edits, for when it reopens; explicitly closing a detail that
opened the panel closes it again. Reopening does not repeat a public graph
lookup: it needs another explicit ask. Mod+I shows the panel with the
assistant in front, brings it forward from another tab, or hides the panel
while the assistant is in view. The tab bar's maximize button gives the
panel the whole page column; the page stays mounted underneath. Details
remember one width and the assistant and the surfaces another, dragged from
the panel's left edge. Below 1024px the panel is a sheet over the page instead: it slides in
from the right (always leaving a 48px strip in view) on a dimmed, softly
blurred backdrop and is modal (focus moves in, the page is hidden from
assistive tech, Escape, the backdrop or its close button hides it, and focus
returns to where it was). While the assistant is not in view,
Settings → Appearance chooses a composer, a slim launcher, or nothing
below the page; either bar takes its own layout space. A tool approval the
assistant asks for is a row in the conversation; it opens the panel once, and
if the panel is closed again a notice under the page keeps it one click away
until it is answered (see [AI](ai.md#tool-use)). The expanded Transactions
table holds the whole column: the panel (unless a detail opened from the
table) and bar stay out of view there, and the shortcut, the title-bar toggle,
Ask about this, a page's handoff, a tool approval or a failed turn (whose error
shows only in the panel) leave the expanded table and show the panel. Opening
or mounting it reads only
local configuration; nothing reaches a provider until the user sends. A conversation belongs to the book it
started in, so opening another book starts a fresh one. "Ask about this" on a transaction row, its detail (which stays open as the
Details tab while the assistant comes forward), or a quarantine cause pins
that record as typed screen context and prefills a question without
sending it; see [AI](ai.md) for the screen-context contract.
The composer's toolbar holds the attach button, one model picker and send.
The picker carries the model, its reasoning level, the speed toggle and the
chat's permission mode in one popover (see
[AI](ai.md#in-app-surface)).

The app talks only to its daemon. It resolves the binary from `--kassiber-bin`
or `KASSIBER_BIN` when set, then the bundled sidecar, then, only in builds with
the `tooling` feature (every debug build and `scripts/run.sh`), the checkout's
`target/release/kassiber`, then `target/debug/kassiber` (`KASSIBER_REPO_ROOT`
can name another checkout). It only looks for those files; it never runs a
probe. Without an explicit data root it runs the CLI's default-root migration
first and starts no daemon if that fails (`src/daemon.rs`).

Report exports open with the system's default app only when the exporting flow
itself just wrote the file, and only for document types Kassiber exports; the
same confirmation can reveal the file in the file manager (`src/toastopen/`).
Attachment and source-of-funds links open in the system browser only as
absolute HTTP or HTTPS URLs with a host and no embedded credentials
(`external_url` in `src/txdetail/actions.rs`); explorer links are built from
the configured explorer and are HTTP(S) only. Headless runs open no files or
links.

The Welcome screen can open existing local books: the native folder picker
(Rune's file dialogs) opens near the default state root, accepts either a
project folder or its `data/` folder, records the approved data root, restarts
the daemon with that `--data-root`, and then lists local books grouped by
books set (`src/onboarding/projects.rs`). Only an approved data root is opened
again. SQLCipher databases must be unlocked before book names can be read
because the internal workspace/profile rows live inside the encrypted
database. Closing an opened folder returns the daemon to the launch's own
root.

Release desktop packages bundle the Rust `kassiber` binary as a
`kassiber-cli-<target triple>` sidecar, with the AI provider broker beside it
([Rust packaging](rust-packaging.md)). macOS keeps it at
`binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin`, the location and
signing identifier of the earlier one-dir sidecar, which the Touch ID
helper's identity check and existing Keychain items rely on; Linux and Windows
ship it flat under `binaries/`. A single native executable has no one-file
extraction and no per-launch dylib re-validation.

At runtime the supervisor prefers `KASSIBER_BIN` when it is explicitly set,
then the bundled sidecar from the app resources (one-dir candidate first), then,
in `tooling` builds only, the checkout build above. A missing binary is a
`daemon_binary_missing` error. The same `KASSIBER_BIN`
override applies to installed-app CLI forwarding.

The shell draws one 40pt title bar carrying the nav toggle, history buttons,
book breadcrumb, alpha chip, and shell actions, over a flush side nav and an
inset page panel (`src/shell/titlebar.rs`). On macOS the window uses an overlay
title bar with the traffic lights in that row; only the bar's empty stretches
move the window and take the double-click zoom, so its buttons keep their own
presses. Full-window surfaces (dialogs, sheets, the lock screen) start below
the bar so it stays reachable. In full screen the traffic lights are gone and
the bar's controls move to the window's edge. On Windows the window has no
native title bar: the same row moves the window and Rune draws the caption
buttons at its trailing edge. Linux keeps its native decorated frame, and the
row is an in-app toolbar. Screens outside the shell
(setup, loading, the error boundary) and the lock and import-restore screens use
the same outline: the title bar row on the chrome and an inset page panel.
Inside it the lock screen centres the wordmark over the welcome screen's type
(title and lead, no eyebrow) and a passphrase field exactly the wordmark's width,
with Open other books set apart at the foot of the panel. That action only
clears the UI session and returns to the start screen; it deletes no books.
While the native Touch ID prompt is open, the form gives way to a single
waiting status and Touch ID guidance. Cancelling or failing that prompt restores
the passphrase field and keyboard focus.

Setup opens on one primary action, creating books, beside its chain artwork:
three-dimensional blocks rasterized onto a coarse pixel grid, in the wordmark's
monochrome and Bitcoin orange. Transactions flicker into the next block, a full
block is found (its stepped outline flashes orange), and the chain moves back
in discrete steps; small pixel cubes above it are the mempool. The artwork is
drawn locally (`src/art/`): it shows no chain data and stays out of the
accessibility tree and tab order. Reduced motion holds it still, and hidden
windows pause both the animation and its timer. A language select beside the
wordmark sets the UI language, the same setting as Settings → Appearance,
before setup starts.

The pixel wordmark appears throughout setup, the lock screen, and the sidebar.
It arrives as falling cells on the welcome screen, decrypts when the lock
screen appears and again on each unlock attempt, and assembles once after
creating books. Folding the sidebar clips
the word down to its K. During the first sync a looping K indicates work;
the adjacent text and progress remain the source of real sync status. These
effects stop in hidden windows and show a static mark with reduced motion.
The app icon keeps the K and Bitcoin-orange wedge on a pixel grid. Standalone
light- and dark-background wordmarks live in `docs/assets/` for reuse.

The setup steps run in one wizard card laid out after T3 Code's welcome
wizard: the identity centred above the card, a segmented step track in the
card's header, and the step in an inset panel. Each step leads with its main decision as option cards; the Sync
step starts with none chosen, so no server is picked for the user. Every card
says what that option lets Kassiber reach and when (the internet when you
sync, this device when you use the assistant, or no connections) and names the
hosts once they are known; the chosen card holds that choice's fields.
Secondary settings, such as update checks, are settings rows with the switch on
the right and the same reach line. Every step's footer says that setup itself
connects to nothing except a server test the user starts. Review lists each
connection first, including what setup leaves off (live prices, exchange
imports, BTCPay, Lightning), then what stays on this machine.

The side nav remembers whether it was folded to its icon rail, and folds by
itself while the window is narrower than 1100px (plus the docked right
panel's width, so the page keeps that room beside it) without changing that choice;
the profile stays on the rail either way. Back and forward also run from
Cmd/Ctrl+[ and ] or Alt+Left/Right. History stays put while typing, with a
dialog open, or while locked. Tooltips and menus
write shortcuts in the platform's notation (`⌘⇧A` on macOS, `Ctrl+Shift+A`
elsewhere).

Dialogs, the palette, and the sync progress card share the command palette's
treatment: the same faint, unblurred backdrop and frosted surface. Sheets, the
right panel's sheet and the Settings modal follow T3 Code instead: an opaque
popover (or page) surface over a dimmed, 4px-blurred backdrop. Dialogs hang
from one anchor below the title bar (8% of the window height, at least 1rem;
`dialog_top` in `src/shell/mod.rs`) instead of centring, so a dialog whose
content changes size grows downward rather than jumping.

Screens share one spacing scale in `src/theme.rs` (page inset, gap between
cards, gap between blocks, card padding). The title bar names the page and is
its heading, so a page does not repeat its name or add an eyebrow line. A page
shows a title only when the title bar does not already say it, such as a
settings section or a wallet's name.
Settings opens as a large modal over the current page, after T3 Code's
settings: a category rail on the left (an icon rail below `md`), a
breadcrumb header with the close button, and the category below; Esc, the
close button or the backdrop returns to the page, which stays mounted
underneath. Each category keeps its `/settings/<slug>` route so links, the
native menu, app search and `kassiber://` intents stay typed; the shell turns
a navigation to one into the modal over the current page without adding a
history entry (`redirect` in `src/shell/mod.rs`). Leaving the page from inside
Settings (Open logs, Resume setup) closes it, as does locking the app, and a
request for Settings made behind the lock screen is dropped rather than shown
after unlocking.

The command palette (Cmd/Ctrl+K) opens as a launcher: the main pages and
everyday actions with their shortcuts, before anything is typed. The
palette, its actions, and the workflow shortcuts stay closed while the app is
locked.

The main window is sized before it opens. The first launch centres a frame on
the primary display (82% by 88%, capped at 1440×960 and floored at the
980×700 minimum). After that the last normal frame and whether the window was
maximized are restored, refitted onto the display the frame is mostly on, or
the first-launch frame when it lands on none; full screen is never restored
and Wayland, which hides window positions, keeps no position
(`src/persist/window.rs`). The frame is kept in the desktop preferences file
(`desktop-ui.json`, or `KASSIBER_DESKTOP_PREFS_FILE`): owner-only, written
atomically, never inside a book, and on macOS kept in `~/.kassiber` until the
default-root migration moves it (`src/persist/file.rs`).

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
startup. The app independently
reads the same owner-only `<state-root>/config/update-checks.json` consent used
by the packaged CLI before spawning anything, fails closed when it is absent,
malformed, or disabled, and rejects any release URL that leaves the official
repository. The app keeps no copy of that consent of its own; every check
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

The Journals screen's **Check reproducibility** runs `ui.journals.verify` and
shows whether the stored journals are identical to a recomputation, stale, or
which tables, wallets and years differ.

Logs → Export also offers **Replay bundle…**, the explicit export of a
[replay bundle](replay-bundle.md): the dialog states what the file holds (exact
amounts, dates, rates and tax settings; pseudonymized identifiers and labels; no
descriptors, keys, credentials or server addresses), needs a tick that the user
understood it and a passphrase entered twice (at least 8 characters), and then
calls `ui.diagnostics.replay_bundle.export`. The daemon writes the encrypted file
under `exports/diagnostics/` and returns its path; the app never names a path.

Transaction detail shows a Transaction flow panel beside its tabs, above them
in narrow windows; the review checklist sits in the header's timeline bar. The
panel uses `ui.transactions.graph` to draw a local, read-only flow view: valued
Bitcoin vin/vout become proportional input/output strands with a distinct fee
leg, reference-only or confidential records can show amountless public
references, and unsupported imports get an explicit empty state instead of a
guessed graph. Liquid legs this book's wallets unblinded during sync keep their
values, including spent inputs and another owned wallet's legs of the same
transaction; conflicting observations stay confidential. Strand widths and
positions follow mempool's bowtie graph unchanged (`src/txdetail/graph/geometry.rs`
ports its calcTotalValue, initLines and linesFromWeights): on Bitcoin the total
is the outputs plus the fee; on Liquid, with unknown legs on both sides, unknown
legs are assumed as large as the average known leg of their side and the
explicit fee is a known output; without any total every leg gets an equal
share; the outer ends of both sides fill the same height; legs past 250 fold
into one "+N more" leg. Widths are drawing only. The graph is drawn in 3D,
inline and in the expanded dialog, as that bowtie in glass after the Bitcoin
Austria artwork lab: one ribbon per input and output, as thick as mempool's
strand, with a block for the coin at its outer end (the fee has none). The
glass piece folds sooner, past 60 legs a side, so a large transaction keeps the
same shape instead of growing into a tall column of hairlines; the list below
keeps every leg. It is seen from one fixed angle and does not turn. The
ribbons meet in one slim glass collar, since a transaction spends its inputs
together; their order does not say which input paid which output. Frosted
ribbons carry no known amount, and hidden values give every leg an equal share
of the same shape. The legend names only the swatches the drawing uses. As on mempool, the inputs and outputs are listed below the
graph, inputs on the left. An input whose coin a row in this book
created, and an output this book later spent, open that row in place: the book's
own history can be walked back and forth like an explorer, without a request,
and the header's Back button returns along the coins followed. Pointing at a
leg in the drawing lights it, shows its card and marks its row in the list;
pointing at a row lights its leg (or the "+N more" strand it is folded
into), and clicking an input or output leg brings
its row into view, unfolding its column if needed (the fee and a folded
"+N more" leg have no row to bring).
The glass view renders on the GPU through Rune's compositor
(`src/txdetail/graph/gpu/`), only when its pose, size, highlight or theme
changed, needs no network, and falls back to the 2.5D drawing when no GPU
compositor is available. The header's amount and the listed legs and
totals follow the app's bitcoin/fiat switch, and clicking an amount flips it;
legs are priced at the transaction's own price and stay in bitcoin without one,
when that price is not in euro, for hidden values, and on another swap leg's
graph. A switchable amount keeps the amount as its accessible name, with the
switch as its description. After a wallet sync returns, a bounded background pass fills
missing graph references through that wallet's backend. Its completion refreshes
local graph queries. The "Look up on-chain" action appears only where a lookup
could still add something: a row with no local graph, or a Bitcoin graph whose
spent outputs lack their amounts, mostly rows without a synced wallet or left
incomplete by the pass's limits or a backend failure; a local Liquid graph is
never replaced. Opening the panel starts no network request, with one
owner-approved exception: while **Settings → Privacy → Fill in transaction
details from the wallet's backend** is on (the default) and offline mode is off,
opening a Bitcoin transaction whose spent amounts are missing asks the backend
its own wallet syncs from, and nothing else, for them (`lookup:
"wallet_backend"`; see [privacy and security](privacy-and-security.md#external-requests-complete-list)).
Such rows show no lookup notice or button, only a subtle loading line, and one
quiet line when the amounts could not be loaded ("Offline — input amounts not
loaded", "Input amounts unavailable: backend unreachable").
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

The Connections detail page for a wallet is the wallet's own record, not a
scoped Overview. Under the header row (refresh and edit/rescan/remove), one
panel holds its balance, a short fact list (UTXOs, transactions with their
date span, last sync and backend) and its monthly booked balance from
`ui.reports.balance_history` (month-end market value when the app shows fiat;
months the wallet held nothing in show as zero, and the line stays in bitcoin
for a range with an unpriced month). When the on-chain inventory and the books
disagree, a card compares the two. A notice appears only when the wallet has
transactions in the review queue or without a price, counted by the
wallet-scoped `ui.transactions.dashboard` read the Transactions page uses, and
links into that page filtered to the wallet. Tabs carry the rest. Transactions is
a table of the wallet's transactions (date, type, description, amount, value,
status) whose rows open in the docked transaction detail; its footer opens the
full list on the Transactions page. Coins groups the unspent outputs by size
and by age since confirmation, with the largest, median and smallest coin;
these are chain facts, not tax lots, so a coin's age says nothing about its
holding period. With values hidden, the size bands show nothing, since their
counts bound the balance; a capped inventory says how many coins it covers. Details holds the connection settings, import runs, and the
Samourai and BTCPay provenance panels. All of these are local display reads;
none starts a sync.

The Coins tab includes a read-only UTXOs table for chain-backed
wallet sources. Refreshing a descriptor/xpub/address wallet updates the local
output inventory, and the table shows current unspent transaction outputs with
outpoint, amount, confirmation state, receive/change branch/index when known,
address or safe label, and source freshness. It shows the rows returned by the
daemon payload a page at a time, reports when that payload is capped, offers
sorting by size, chain date, confirmations, or outpoint, and can open the UTXO's
transaction in a configured/public explorer after the same privacy warning used
by transaction detail explorer links. The table is
inventory-only: there is no spend, PSBT, signing, broadcast, coin-selection, or
freeze action.
Unsupported file/BTCPay/Lightning-style sources show an unsupported state, and
Liquid sources show an unblind blocker unless Kassiber has descriptor material
that can unblind outputs locally.

Overview leads with one readiness pill that names what holds the book back
(sources, stale journals, open review, custody gaps, missing prices) and opens
where that is dealt with; stale journals it reprocesses in place. Then three
tiles: BTC price, the bitcoin held and in how many wallets (observed, so it
carries no status and opens Wallets), and what it is worth with its change
against cost basis, both shown whichever currency the app displays. The
latest transactions sit beside the activity chart once the page is about
64rem wide (measured on the page, so a docked assistant counts), as many whole
rows as fit the chart's height, and under it otherwise; the list never
scrolls inside the page, and Show all opens Transactions. A row names its
status only when it asks for something. Holdings by Source runs across the
page below as a ranked list, largest first: each source with its rail's logo,
its value (whole currency units in fiat) and its share of the bitcoin held.
Past eight, the smallest fold into one Other sources row. Its header leaves an
incomplete cost basis to the tile and the chart.

Overview, Wallets, the book-set overview, and the Reports metric strip read the
daemon's `fiat.completeness` block (see [the daemon reference](daemon.md)) instead of
trusting cost basis as-is. While it is incomplete, the fiat portfolio card turns amber
and shows an Incomplete or Outdated badge instead of the "vs cost basis"
percentage. It names the BTC without cost basis and links to Review, or to
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
readiness pill. Book-set rows get a per-book badge, and
the fiat total says how many books are incomplete. Reports marks cost basis,
gain/loss, and indicative tax as Provisional while journals are stale or
quarantines, custody gaps, or missing prices exist, and names the cause with a
link to the page that resolves it. Reports copy stays English under the
deferred reporting-language policy in [i18n.md](i18n.md).

Review sits in the main nav and collects what waits on the owner, each item
decided where it is listed. **Transfers** (`/transfers`; `/swaps` and
`/custody-gaps` redirect to it) sits in the main nav too and is built from
the same pieces as Overview, Transactions and Wallets: a first row with the
status selector, Recalculate journals while the journals are stale, and the
actions, Link manually first, a stat strip (linked
transfers, and what moved between wallets in the display currency), and one
card per status, picked with the status selector at the top left of the page
(drawn like the period selector on Transactions, each status with its count): **Needs you** (held questions
and open suggestions; it opens first while it holds anything), **Linked**,
**Unlinked by you** and **Set aside**, the last two only while they hold
anything. Clicking a linked transfer opens it in a panel beside the table
when the page is wide enough, the row marked, and as a dialog otherwise. Link
manually docks in the same place (one panel at a time; a transfer opened while
a link is being made comes as a dialog that offers no link of its own, so the
link is not lost), and the lists size themselves by the room beside the panel.
The link is one tree that moves between the panel and the dialog, so its
picks, search, answer and running check survive the page changing width; the
panel takes focus when it opens and gives it back to where it was opened from. **To sort out** asks about each held move once: a send
whose automatic link the owner removed and that the journals now hold reads
as the question it leaves open ("You unlinked this ₿… send from A. The coins
arrived in B, which you own, so Kassiber can't book it as a payment. What was
it?") with the answers the core allows: link it back
(`ui.transfers.dismissals.delete`), choose a different receipt with Link
manually, or, with developer tools on, describe it as a custody component. The
core has no decision that books such a send as a payment or its receipt as
income, so none is offered. The part of a send the owner linked that no
receipt covers yet is asked about the same way: the core's residual hold
(`reviewed_residual_suspense`, named by the hold's `review_pair` action) on a
pair with `shares_legs` whose send has more left than a fee could be, over 1%
of the transaction and over 2,500 sats, the matcher's own fee limits. Its
answers are linking the receipt the rest arrived in, which anyone can do, and
the missing-wallet review the hold already had (developer tools; without them
a line says where that is). A smaller shortfall on an ordinary link, such as
an exchange fee taken off the amount, is not a part waiting for a receipt: it
stays the missing-wallet hold it was, and the transfer's details name it as
the fee between the two sides. A hold that names a pair for another reason
(an old fee allocation to repair) is not read as a rest. A
hold a missing-wallet review explains is shown there read-only; reviewing it
stays in the developer-gated advanced tools. Needs you holds the open
suggestions (the same list as Review's; with none, one line whose info icon
says what Kassiber checks, with Check again; several suggestions can be ticked
and set aside in one step, with one Undo); Set aside lists the suggestions the
owner set aside, each with Restore; Unlinked by you lists the pairs the owner
unlinked whose legs are free again (from `ui.transfers.list` with
`include_unlinked`), each with Link again and Link differently; and Linked has
one row per transfer linked automatically or by the owner. Every row's menu
opens its send and its receipt on Transactions (`/transactions?tx=<id>`); for
two sides that count on their own (unlinked or set aside) it also leads to
each transaction's tax tab, where saying what it was (a payment, income) is
decided with the core's preview. The page menu's Look for transfers again
recalculates the journals, which runs the matching again. A row shows the date, the route with
each wallet name cut in the middle so its last words stay, the amount (exact
bitcoin with the leading zeros dimmed, or the value at the sending
transaction's price when the display currency is fiat), the sending
transaction's network fee in sats (or a swap's recorded fee), its type
(Transfer, Peg-in, Peg-out, a Lightning or chain swap) with Taxable as the only
coloured chip, and how it was linked: an icon for automatic, a coloured one
for by you or by a rule, the reason behind an info icon aligned down the
column. A link that is one transaction seen by two wallets cannot be unlinked;
its menu has no Unlink. A transfer whose send is linked only in part reads
Part linked in the Linked column, the amount left behind its hint, and its
details say the same and offer Link the rest. While the journals recalculate, the
suggestions and counts stay as they were before the decision and Unlink waits,
so a read of stale journals never offers moves that are already linked.
Narrow, the fee and the time give way first. Prices come from
the core: `fiat_value` and `fee_fiat_value` on `ui.custody.lineage.snapshot`
rows and per-leg `fiat_value` on `ui.transfers.list`; an unpriced move says
Unpriced. Clicking a row opens the transfer: both legs with their transaction
ids, the difference between them and how it was linked, with Unlink. Unlinking
shows what changes first: the core's dry run for an automatic link, or what the
core does to a pair, worded from its tax treatment (a taxable pair already
counted as a sale and a purchase, so its tax figures stay). What happened is
then said where it happened: on the Linked list, a strip at the transfer's row
(or where the row was) with Undo that stays until it is closed, read out
through the page's one live region; on any other
list, a short toast with the same Undo, never both (Undo restores an unlinked
pair with its terms at once; it waits while the journals are processed). After
a link that leaves part of its send, the note offers Link the rest. Nothing is
repeated as a banner after a reload. **Link manually** is one
screen: the send and the receipt side by side on top, and under them either
the list for the side that is still open or, once both are chosen, the check.
It usually opens with one side known (a suggestion's Review… or Choose another
receipt, a held question, Link differently, a `link=<transaction>` deep link
from Transactions or Review), so the usual path is one pick and one
confirmation; Link again opens on the check with both. A transaction that is
not in the book says so and leaves both sides open. The list is a table (when,
wallet, amount, difference to the chosen side, what the transaction is booked
as today) read from `ui.transactions.list`, at most 200 rows and said so when
there are more, with one search field (wallet, txid, note, or an amount, which
brings the nearest amounts first) and one time choice (a window around the
chosen side, three days at first, or a year when nothing is chosen yet). When
the time leaves the list empty, or with nothing in it that fits the chosen
side, it says what it looked through ("No receipt within 3 days of the send")
and one button widens it a step: a week, 30 days, any time. Every row opens
its transaction on Transactions. The
matcher's suggestion and a transaction with the chosen side's own txid come
first, then amounts and times that are close; an amount that is no fee (more
arrived, over 10% missing, another asset), the same wallet and a receipt
before its send are listed after them and coloured, never hidden. Linked and
excluded transactions stay listed last and cannot be picked, except a
transaction whose links leave part of it open (the same rule as above:
`shares_legs`, and more left than a fee could be) when the other side makes an
ordinary move within one asset with it; a swap, a peg or a Lightning hop
cannot take the rest of a transfer, since the core keeps those one to one,
and such a candidate is listed with that reason and cannot be picked. A send that arrived in several receipts, or a
receipt fed by several sends, is linked part by part, each link an ordinary
pair with its own dry run, and the list and the check are then about the part
that is left ("₿0.4 of ₿1 still unlinked"). Selecting several candidates in
one step is not offered. Arrow keys move
between rows, Enter picks. The check names the difference in a sentence, says
what the link counts as, and shows the core's dry run (`ui.review.plan`): holds
it clears or creates, whether reports wait, and each tax year that moves as a
sentence. Within one asset the core links only as the owner's own coins moving
with their cost basis, so that is stated and nothing is asked. Between two
assets (BTC and L-BTC, or a swap into another asset) the owner answers "Was
this a sale?" with the two consequences spelled out; the answer starts from
the pair's old terms on Link again or from the matcher's proposal for that
exact pair, and otherwise nothing is assumed and the dry run waits for it.
Which answers the book allows is read first (`ui.transfers.pair_options`):
where it counts the move as a sale only, that is stated with the reason
instead of asked; anything else the read says leaves both answers on offer
and the dry run decides. The
kind (peg, Lightning swap) is read from the two sides or taken from the
matcher or the old pair; it is not a choice here. Everything worth a second
look (more arrived than was sent, amounts that don't match, over 1% within one
network, a receipt dated before its send, the same wallet, a side that is
already linked or excluded) is listed once in one warning, and with any
serious one, or when the link itself puts transactions on hold, the button
reads Link anyway. A large shortfall within one asset is one warning that
names both things it can mean (not the same coins, or the rest arrived in
another transaction and can be linked next), and holds a link creates are
said once. The dry run shows how long it has taken, waits while the
journals recalculate, is reused for the same change and book version, and is
joined instead of started again when the dialog is reopened while it runs. A
refusal by the core is put in plain words above the core's own; another
failure offers Try again. If the book changed between the check and the
confirmation, nothing is linked and the check runs again. After linking the
dialog or panel closes and the note at the row (or the toast) offers Undo
(`ui.transfers.unpair`).
The screen and its dialogs say link / Verknüpfung; the CLI keeps `pair`. With
developer tools on, the advanced tools (missing wallets, components, scan
coverage) are a secondary section of the same route; linking rules and saved
views open in a sheet from the menu. A rule reads as one sentence (what it
matches, then the kind and treatment it records), marks the suggestions it
matches with a Rule chip and fills in its terms when they are linked; deleting
one asks first. The
pages with the remaining full tools (Journals, Reconcile and
Source Funds) sit under Extras above the calculators; the fold opens and is
marked whenever one of its pages is open, however you got there.
**Moves between your wallets** comes first, because linking a move often
clears a quarantine cause with it. **Can't be booked yet** holds the
quarantine causes, as [custody resolution](custody-resolution.md#why-a-transaction-is-quarantined)
describes, with the journal freshness warning above them. Each cause card
keeps the full instruction behind its info button where its buttons already
say what to do. Each card lists its
transactions folded to three, with one Show more that shows the rest of the
loaded page and then loads the next. A cause of approximate prices held for
review offers **Accept these prices**, which accepts all of them in one
previewed step (`accept_coarse_prices`). A price cause also offers **Enter
prices**: a field per row for the receipt's total value or the price per
unit, in the book currency (`summary.fiat_currency` of
`ui.journals.quarantine`). Input is read in the owner's number format; a
leading zero always starts a decimal, and the preview lists each price as it
was read. Rate fields and their confirmation name each transaction's own
asset, including when a cause holds several assets. Up to 50 typed prices go
through one previewed step as
`price_override` operations, each saved as exact and revertible from the
transaction's history. **Moves between your wallets** lists the transfer
and swap candidates from `ui.transfers.suggest` as table rows with the same
edges in every row: the date, the route between the two wallets with their
rails' logos (chain, Lightning, Liquid), one amount with the fee under it
(instead of the fee, what arrived, for a swap into another asset or when no
positive fee is left, as when more came in than left), and what linking records:
a plain transfer quietly, a Bitcoin swap or a swap into another asset (Different asset) as a chip with its default
treatment (tax-neutral carrying value or taxable), with how sure the match is.
The rows size themselves by the list's width, not the window's, so a docked
assistant gets the narrow layout: two lines, with the route and amount above
the date, kind and buttons. A long wallet name is cut short with the full name
on hover.
The decision status masks wallet names while sensitive details are hidden.
An exact match without competitors has Link, which links it with its default
kind and policy, or a matching rule's; a likely match or one with competitors
has Review…, which opens Link manually on its send with the alternatives as the
choice. A menu holds Not my transfer, which sets it aside, and Choose another
receipt. Exact candidates with carrying-value policy, no competing
match and no ownership proof can be linked in groups of up to 50.
`ui.review.plan` binds their exact transaction IDs, policies, input version and
book to the confirmation; `ui.review.apply` refuses stale confirmations and
applies the preview atomically. Taxable and ownership-graph candidates stay
one at a time. A decision can be undone for 20 seconds
(`ui.transfers.unpair`, or `ui.transfers.dismissals.delete` for a dismissal).
Once the owner stops deciding, the journals recalculate. That is local and
makes no network request. The queued refresh survives navigation, including a
decision response received after leaving Review. It carries the originating
book scope and is abandoned after a book switch, leaving that book visibly
stale until it is processed. Until it finishes, the list keeps its earlier
reading less what was decided, because a read of stale journals lists more
candidates than are open. One number counts what is left: `ui.review.badges`
`review`, which is `quarantine_attention` plus the cached transfer candidates
(an ownership-graph move counts once, under its held transaction).
The sidebar's Review badge, the bell and search show it, in red while a
quarantine holds rows back from the reports, and so does Overview's readiness
pill while a quarantine is open and nothing more pressing is. Stale journals
with nothing else open show a dot. The side panel's Review queue counts
causes and transactions as the screen does and opens Review; a dismissal is
stored with an English reason whatever the app's language, like accepted and
entered prices. `/quarantine` and `kassiber://quarantine`
open Review, and the native menu's Review item keeps Quarantine's shortcut.

Reports shows one tax year from `ui.reports.capital_gains`: whether it can be
filed, its result and how that result is built, and the files that carry it.
It lists no rows; those stay on Transactions. Proceeds, cost basis, and
gain/loss come from the payload's full-year `totals`; the gain/loss tile names
the income and the tax-free Altbestand gain (from the breakdown's `tax_free`
group) it includes. For an Austrian EUR book the payload carries the core's
indicative tax (`indicativeTax`: 27.5 % of KZ 172 + 174 − 176, with KZ 801
listed and never estimated) and the screen shows it with the core's caveats
behind the tile's info icon. A year before 2022 has no crypto Kennzahlen
(`e1kvApplies: false`): the screen states only KZ 801, shows no estimate,
says so in the readiness notice and Rules, and offers no E 1kv tax report.
Rows dated before 1 March 2022 have no crypto field in any year; a 2022 with
such rows (`rowsBeforeCryptoKennzahlen`) says so before export, withholds the
estimate and offers no E 1kv tax report. Generic books get no estimate, and a region the
screen does not recognise reads as generic, never as Austrian. While the
completeness read is still loading, the readiness pill says so and nothing is
marked Provisional; a failed read or one without completeness stays
"Completeness unknown". The page also reads the core's report barrier
(`ui.report.blockers`, the same maintained read as the report): its own checks
name the specific cause first, but the year never reads "Ready for reports"
while the core answers `ready: false`, has not answered, or failed to answer;
the notice then shows the core's first blocker (a pending Silent Payments scan,
for example) and how many more there are. When a readiness notice names a
blocker, the header pill is hidden rather than repeating it.

The breakdown card states the result the way the report does. An Austrian book
lists its E 1kv fields and KZ 801 on the E 1 with the core's amounts, then what
stays off the return: tax-free Altbestand, tax-neutral swaps, and rows without
a field. Rows without a field may still be taxable, so they show in amber, mark
the figures Provisional and keep the year from "Ready for export". A generic book lists short-term gains, long-term gains, and income.
Each row's transaction count links to Transactions filtered to exactly those
transactions (`txids`, from the payload's `breakdown`); a group of more than 128
opens the whole tax year instead, since the Transactions filter takes no more:
the year's own bounds in the book's zone (its `taxYear`, else the country's
calendar year), computed with the core's zone calendar
(`crates/tax-countries/src/calendar.rs`, compiled into the app) and passed as
the list's `since`/`until`, shown as a clearable date-range chip. How
a single gain was calculated opens from the Calculation row on the
transaction's Tax tab, which `ui.journals.events.list` feeds with an
`explanationReference` for realized disposals and income.

A Rules card lists the book's country, its stored gains algorithm, the core's
rate (none when the core gives none), the Altbestand rule for an Austrian book
or the holding rule for another (no long-term rate, held more than a year past
the anniversary of acquisition, or a generic book's own day count), and the
tax-year boundary with the zone the payload's `taxYear` carries, and links to
Books, where they change. The Files panel writes the
tax report as PDF, XLSX, or CSV (the year's E 1kv for an Austrian book, the
complete history for a generic one), the bookkeeping handoff, the summary PDF,
and the audit package; the ledger's own exports stay on Transactions. The
summary's wallet scope and cover page and the audit package's scope and
contents fold under their rows. The outcome of the last export stays at the
top of the panel.

Privacy Mirror is the dedicated desktop page for local privacy linkage. It
reads `ui.reports.privacy_mirror` and shows exposure summary, adversary cards,
wallet/transaction/UTXO rows, timeline, coverage, unknowns, evidence
drilldowns, and a PSBT/what-if panel. The page
is local-only, read-only, advisory-only, shows degraded states instead of
standing reassurance badges, and uses the normal desktop daemon allowlist; it
does not sync, sign, broadcast, select coins, or mutate accounting data.

Settings -> AI providers displays each provider's API-key presence plus storage
location/state. Saving provider metadata does not include the raw key in the
create/update request; when the API-key field is filled, the form sends the
narrow `ai.providers.set_api_key` daemon kind and receives only redacted
metadata back. A connection test sends a newly typed key for that one test
only, else the daemon uses the stored key, and only on the stored base URL's
origin (scheme, host and port).
The provider form, its presets, posture and API format are described in
[the AI reference](ai.md#adding-a-provider-in-the-desktop). Settings can
move an existing provider key between `sqlcipher_inline` and the native store
with `ai.providers.move_api_key`. macOS production-signed builds may default to
Keychain; unsigned/ad-hoc/unknown macOS builds keep Keychain opt-in
experimental copy because rebuilds or app identity changes can prompt again.
Windows uses user-scope Credential Manager/DPAPI when available. Linux falls
back explicitly to `sqlcipher_inline` when no desktop secret service, D-Bus
session, or unlocked collection is available. There is no
plaintext fallback and no remember-unlock behavior.

The same Settings screen carries **External agents**, off by default and
disabled while AI features are off. Its **Allow agents** switch sits above what
agents may do (**Read**, **Read and operate**, **Accounting decisions**) and a
**Restrict results** switch, with copy saying what the selected grant allows; a first
enable preselects accounting decisions with full access, and a recorded choice
is kept. Turning it on records the switch and choice through
`ui.agent_access.configure` and shows a `claude mcp add` command, plus a
copyable JSON config, pinned to the current book. The desktop also mirrors the
AI features switch into that preference, writing only when it changed, so
`kassiber mcp serve` refuses agents while AI is off; see [MCP](mcp.md).

Settings -> Desktop -> Developer tools carries the early-stage-features switch,
which is off by default and gates the surfaces that are not finished yet. With
it off, Network Monitor, Logs, and Settings -> Lightning disappear from the
side nav, the Settings modal's rail, and app search, while Activity, Custody Gaps, Exit
Calculator, and Privacy Mirror stay in the nav as greyed-out, inert rows. The
nav treatment is a signpost, not the barrier: every one of those routes carries
a guard that redirects to Overview (Settings -> Lightning stays shut, and falls
back to the default category if the switch turns off while it is open), the
native menu's Logs item is disabled, and
a native-menu or `kassiber://` intent for any of them lands on Settings ->
Developer tools with the early-stage switch highlighted instead of a silent
bounce (an intent for the assistant with AI off highlights the AI switch the
same way). The
choice is stored in the desktop preferences file, so it survives updates;
`DEV_LOCKED` and `DEV_HIDDEN` in `src/shell/mod.rs` hold the route lists.

The title bar's bell opens the notification list (`src/shell/notifications.rs`)
in three sections:

- **Needs attention**: unresolved conditions, blockers first. A failed book
  refresh and a failed journal run each have their own row with Retry, which
  repeats the run as it was (a rescan stays a rescan) and sends the open
  book's ids as `expected_scope`, so the daemon refuses it if another book
  opened in the meantime. Journals waiting to be processed and the Review
  count come from `ui.review.badges`; sources that failed or block reports
  come from `ui.freshness.status`, folded into one row whose details group
  each source's label and last error by cause. Sources that do not reach the
  previous calendar year's end get one row that opens the coverage view
  ([coverage](#sources-live-sync-and-file-snapshots)); it is a local read.
  Conditions about saved unlock methods open Settings → Security. Each row
  says what is wrong and what it means and offers one action from a closed
  list. Conditions are rebuilt from those reads, so a row clears when a later
  read shows it resolved; rows cannot be dismissed.
- **Running**: a book refresh or journal processing in progress. A running
  book refresh reopens the sync card.
- **Recent**: up to 20 finished results from this session, newest first, filed
  under the book that was open (nothing is filed while the open book is
  unknown); Clear recent empties the open book's entries only.

The bell's badge counts the Needs attention rows; it turns amber, or red while
one blocks reports. Rows render in the current language and privacy mode:
details pass the backend's secret floor, Hide amounts withholds free text that
contains digits and masks source names that do, and Hide all shows no details,
while actions keep working. The locked title bar has no bell.

Feedback on what the user just did (saved settings, attachments, edits,
setup receipts, refused or failed foreground actions) never enters the list as
a condition: errors in a form stay beside the form, and the rest are toasts.
Toasts show at the bottom right for about six seconds (ten when they carry
Open or Reveal actions for a written file), stay while the pointer is on
them, close with ×, and are announced to screen readers; equivalent
confirmations replace each other and count repeats. Toasts keep no history,
are hidden while Kassiber is locked, and are never sent to the operating
system.

Notifications live in memory only. Nothing of them is written to the desktop
preferences file, and loading an older file drops any notifications a previous
version stored; a restart starts with an empty list and rebuilds conditions
from the book after unlock.

OS notifications are opt-in: **Settings → Privacy → Desktop notifications**
(off by default) lets background outcomes reach the operating system's
notification center. The switch is the only thing stored
(`desktopNotifications` in the preferences file). Eligibility is decided in one
place, `eligibility` in `src/settings/effects.rs`:

- the switch is on;
- the event is a finished book refresh or journal run, or one that failed
  (the results the notification center files);
- the window is not focused (an unknown focus state counts as focused);
- the same event was not delivered in the last 30 minutes, and nothing was
  delivered in the last minute.

Foreground receipts, toasts, progress, recovery, lock and unlock, and the
unchanged Review or journal backlog never reach the OS.

The text is fixed in Rust (`src/native/notify.rs`): the title "Kassiber" and
one of "Background work needs your attention." or "Background work has
finished." (Austrian German when the UI is German), so no amount, name,
address, txid, server, path, or error text reaches the system's notification
store. A native backstop spaces deliveries at least ten seconds apart. macOS
posts through `UNUserNotificationCenter` (Rune's `rune-notify`) from a bundle,
and through `osascript` when run unbundled; Windows shows a toast under the
installer-registered AppUserModelID; Linux uses the freedesktop notification
service over D-Bus (`gdbus`, else `notify-send`). A click only brings Kassiber
to the front, as the operating system does; the bell shows what happened. A
click never syncs, approves, retries, opens a file, or unlocks.

Platform limits: on macOS the OS asks for permission, if at all, when the
first notification is posted, which can only happen after the switch is on.
Linux notification servers generally do not activate the app on click.
Delivery adds no network request, poll, or probe; it only reacts to work that
was already authorized. Headless runs record notifications instead of showing
them.

### Clipboard, screen sharing and OS backups (Rune desktop)

The Rune desktop's handling lives in `crates/kassiber-desktop/src/osleaks/`. The
privacy rules are in
[privacy and security](privacy-and-security.md#operating-system-surfaces).

- **Copies** go through `osleaks::clipboard::copy` (`settings::effects::copy`
  delegates to it). Never call `Tx::copy` directly. macOS writes the
  pasteboard on a `kassiber-clipboard` worker thread: current host only, plus
  the concealed and transient nspasteboard.org types. Windows and Linux use
  Rune's `copy_secret`. **Settings → Privacy → Clear clipboard after copy**
  (`clearClipboard`) sets the 30-second clear. Locking the session and
  quitting clear a copy that is still held.
- **Hide amounts while the screen is shared** (`hideAmountsWhileScreenShared`
  in the preferences file, default on) sits in Settings → Privacy → Screen & clipboard.
  While the app holds amounts hidden, a status line there says so, and a toast
  appears when hiding starts. Detection is macOS only (see the privacy
  reference); on other systems the switch is disabled and says why.
- **Keep unencrypted books out of Time Machine**
  (`excludeUnencryptedFromBackups`, default on) sits in Settings → Data →
  Backup & restore, with one line describing the open book's state. A
  synced-folder warning appears there while an unencrypted book is in a
  cloud-synced folder. On Windows and Linux the row is replaced by a note for
  unencrypted books.
- Tooling runs (`persist::file::tooling_run()`: tests, `--scene`, `--snap`,
  `--script`, `--tree`, the click-through) never write the clipboard or set
  file attributes. They record the plan instead (`osleaks::clipboard::recorded`,
  `osleaks::backup::recorded`). Scenes: `osleaks-privacy*`, `osleaks-data*`.

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
`/usr/bin/kassiber` command; the desktop package depends on the display, input
and Vulkan libraries the app draws with, while `kassiber-cli` does not.

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

## Sources: live sync and file snapshots

Kassiber has two kinds of sources, and the desktop says which one each wallet
is, how far its history reaches, and the one action that brings it current.

- **Live** sources keep themselves current when the user asks: descriptor,
  xpub and address wallets (Electrum, Esplora, Core RPC), Silent Payments,
  Liquid, Lightning nodes, BTCPay, and exchange APIs with a read-only key.
  A live source covers history through its last *successful* sync.
- **File** sources are snapshots: an exchange CSV, a generic ledger, Wasabi or
  Phoenix exports, OCR drafts. A file covers the span of the rows it held
  (every row read, including rows already in the book), never the time it was
  imported. The user downloads a newer export to move it forward.
- **Hybrid** sources are exchanges with both: the API keeps recent activity
  current and a CSV covers older history the API cannot return. The exchange
  module may declare how far back its API reaches (`api_history_limit`;
  none declare one yet).
- **Manual** ledgers (a custom or generic-ledger wallet with no remembered
  file and no import run, untracked wallets) have no notion of currency; they
  show their transaction span only. A remembered `source_file` or any import
  run makes a wallet a file source whatever its completeness class says.

### Coverage model

`kassiber_wallets::coverage` computes each source's coverage from the book
alone, plus the local metadata of the file the wallet remembers importing
(`symlink_metadata`: a regular file's size and modification time; a symlink,
FIFO or device yields nothing). It never opens or reads that file and never
opens a socket. Per source:

| Field | Meaning |
| --- | --- |
| `source_mode` | `live`, `file`, `hybrid`, or `manual`, from the source's configuration: an exchange API key makes it live (hybrid when it also has a remembered file or file import runs; the run an API sync records names no file and is never file evidence), a remembered file or import run makes it a file source, the live completeness classes stay live, and a wallet with none of these is manual. |
| `covered_from` | Live: the declared scan start or first transaction. File: the earliest row its import runs held. Hybrid: the earlier of both. |
| `covered_through` | Live: the last successful sync. File: the latest row its import runs held. Hybrid: the later of the API sync and the files' last row. Rows outside import runs (entered by hand, synced from an API) never extend a file's reach. A run's range counts only as far as the rows that import runs of the same wallet and importer (`source_format`) created still reach: rows removed by a rollback or a book reset take that coverage with them, also for runs that only re-observed them. Rows entered by hand, synced from an API, or created by another importer never support a file range. |
| `freshness_state` | `current`, `behind` (covered through a day before the period end and no attestation covers it), `never_imported`, `sync_failed` (a freshness source or the stored exchange sync failed after the last success, reason `last_sync_failed`; or the last sync stored degraded results that block reports, such as a partial silent-payment scan, reason `last_sync_incomplete`, until a complete sync clears it), or `unknown` (manual, or no dated rows). |
| `reason`, `action` | A stable code and the one action that fixes it: `sync`, `import_newer_file`, `sync_or_import`, `review_sync_error`, or `none`. |
| `attestation` | The user's statement "no activity after *date*" or "account closed on *date*", with a note. Both say nothing happened after the date. A period's missing interval is (covered_through, period end]; the statement closes it only when its date is on or before the covered end's day, and then for every later period end. A later date leaves (covered_through, date] unattested: the source stays `behind` with reason `attestation_leaves_gap` and `attestation_gap` names that interval, ending at the period end when the date is later. A statement never stands in for a source with nothing covered. It never creates transactions, balances, or tax facts. |

The period end defaults to today; reports pass their own. Comparison is by UTC
day, so a wallet synced earlier today is current for today.

Every successful file import records a run with what its file held
(`import_batches.first_row_at`, `last_row_at`, `rows_read`, `file_sha256`,
`file_size`; see [imports](imports.md#import-runs-and-rolling-one-back)), also
when it added no rows, so a re-import counts as evidence that the file was
seen. Older runs have NULLs; coverage then uses the span of the rows those runs
created, never other rows of the wallet. Rolling a run back also clears the
ranges of runs of the same file (same `file_sha256`) that created no rows of
their own, and a book reset clears every run's range: such runs only
re-observed rows that are gone. "This file may have changed since the last
import" is advisory: it compares the remembered file's metadata with the
newest run (a different size, or a modification after that import), which a
touched but identical file also trips. Every source that remembers its file
(`file.source_file_remembered`, present only then; the path never leaves the
backend) offers "Check file contents", whether or not the advisory shows. It
asks for `check_file` with one `wallet`: the read hashes the remembered file
under the same bounds as an import and compares it with the newest run's
`file_sha256` only, reporting `file.source_file_check` as `same`, `different`,
or `unknown` (no file, refused or over the limit, or a newest run with no
hash, as for a file too large to fingerprint; an older run's hash says
nothing about the last import). Other reads never open the file. The book stores the size, not the file's
time, inode or device: recorded sessions require deterministic stored bytes.
Hashing happens only during an explicit import or check, streamed, up to
256 MiB, after a regular-file check that refuses symlinks and FIFOs. Every exchange API sync,
CLI or desktop, stores its outcome on the wallet it ran for, also when no
`--wallet` named it (`config_json.exchange_sync`: time, status, counts, and for
a failure only an error class: `auth`, `rate_limited`, `offline`, `network`,
`protocol`, `validation`, `config`, or `other`). No provider message, hint, or
response text is stored; the desktop explains the class in its own words.
Books written before classes kept the provider's message and hint there: every
open rewrites such a record to its class, with `secure_delete` on so the old
text does not survive in freed pages, then checkpoints the WAL into the
database file and truncates it. Only a completed truncate ends the cleanup: a
reader holding an older snapshot keeps the old pages alive, so after a few
short tries the cleanup stays pending (the book stays usable) and each later
open retries it ([compatibility](database-compatibility.md)). A
success also stores `last_synced_at`, so the sync card, coverage, and completeness survive a
restart. API syncs write no import batch: the next sync would re-import any
rolled-back row by its exchange ID, so a batch could not be undone; the CLI
behaviour is unchanged.

Reads: `ui.sources.coverage` (`period_end`, `wallet`, `check_file`) and
`wallets coverage` (`--check-file` needs `--wallet`);
`ui.overview.snapshot` embeds each connection's coverage for today (without
scan-bound warnings, so the hot read skips the per-row balance walk) and derives
its `status` from it (`synced` only for a current source, `behind`, `error`
for a failed sync, `idle` otherwise). Write: `ui.sources.attest_coverage`
(`wallet`, `kind`, `date`, `note`, or `clear`) and `wallets attest-coverage`,
behind the assistant's consent gate like every mutating tool.

### Connections list

The Coverage column replaces "Last activity", and the metric that counted
wallets with any transaction as "Up to date" now counts current sources.

```
 Connections                                           [ + Add connection ]
 ┌───────────────┬───────────────┬────────────────┐
 │ Total balance │ Current       │ Needs action   │
 │ 1.2345 BTC    │ 4 of 6        │ 2              │
 └───────────────┴───────────────┴────────────────┘
 [Kind ▾] [Status ▾] [ Needs action ]                       6 connections
 Name            Kind        Coverage                              Balance
 Cold storage    Descriptor  ● Live · synced 2 h ago               0.80000
 Kraken          Exchange    ● API + CSV · synced 1 d ago          0.20000
 Bitpanda CSV    Exchange    ▲ File · covers through 30 Jun 2026   0.10000
 Old Strike      Exchange    ● File · no activity after 2025       0.00000
 Lightning node  Lightning   ✕ Sync failed                         0.13450
 Ledger import   Generic     ○ Manual entries                      0.00000
```

"Needs action" filters to `behind`, `sync_failed`, and `never_imported`; the
shell's Needs attention condition "N sources need action for 2025"
(`sources.behind`: sources that need action for the previous calendar year's end,
the same three states, from
the local read when the shell mounts, no timer; it clears when a later read
finds none) opens the list with that filter and period.
With a period the list reads that period's coverage; until the read answers its
rows show none, and a failed read shows an alert with Retry instead of
letting today's coverage stand in for the period.

### Connection detail: Coverage card

The card sits under the balance panel, above an exchange's sync card, in its own
module (`crates/kassiber-desktop/src/wallets/sources.rs`).

```
 ┌ Coverage ─────────────────────────────────────────────────────────────┐
 │ File snapshot · Bitpanda CSV                         ▲ Behind for 2026 │
 │ Covers 3 Feb 2024 – 30 Jun 2026 · 3 imports, last 2 Jul 2026          │
 │ Updated file found — import it?                                       │
 │ [ Import newer export ]  [ No activity after… ]                       │
 │ In Bitpanda: History → Export → CSV, all assets, full range.          │
 └───────────────────────────────────────────────────────────────────────┘
 ┌ Coverage ─────────────────────────────────────────────────────────────┐
 │ API + CSV · Kraken                                        ● Current    │
 │ API keeps recent activity current; CSV covers history before 2023.    │
 │ Synced 2 h ago (12 new, 340 fetched)                                  │
 │ [ Sync now ]  [ Import older CSV ]  [ No activity after… ]            │
 └───────────────────────────────────────────────────────────────────────┘
```

After an import the card reports "N new rows, M already present, now covers
through *date*", or "Nothing new — every row in this file is already present"
when the run added no rows. "No activity after…" defaults to the covered end's
day; a later date shows which days stay unattested. Scan-bound warnings appear as plain sentences
("History before the first transaction is unknown", "Never synced").
"No activity after…" opens a confirmation that says the statement only closes
the gap from its date on; clearing it asks again.

### Add connection: key or file

Where a provider offers both an API and a CSV (the catalog pairs them:
`file_source_id` in `crates/kassiber-desktop/src/wallets/add/meta.rs`; Binance today),
the modal asks first:

```
 Kraken
 ( ) Connect with a read-only key — keeps itself current        Recommended
 ( ) Import a file — a snapshot you re-import later
 Connect for ongoing use; import a CSV for older history the API cannot
 return. You can add the other one later to the same source.
```

A provider only needs catalog fields to get this choice. When a CSV wallet of
the same exchange already exists, adding a key offers to attach it to that
wallet (`ui.connections.exchange.create` with `wallet`), which makes it a
hybrid source. Rows deduplicate within one wallet by external ID and
fingerprint, so CSV and API rows match only where both carry the same
transaction ID scheme.

### Refresh all and reports

The title bar refresh (its button and menu, Cmd/Ctrl+R and Cmd/Ctrl+Shift+R, and the
command palette's Refresh and Full rescan) is an explicit user action: after the book refresh it also
syncs every exchange wallet with a key, one `ui.connections.exchange.sync`
each. The sequence belongs to the book the click was made in: each request
carries that book's `expected_scope`, which the daemon refuses for another
book, and the sequence stops as soon as the book or daemon session changes. Menu book refreshes, background freshness, startup, and display reads
never contact an exchange ([privacy](privacy-and-security.md)).

The Reports page reads `ui.sources.coverage` for the selected period's end.
Sources that are `behind` (or failed, or never imported) for it appear as a
readiness warning naming each source and its action; the report is marked
provisional, as missing fiat rates mark it, and export is not blocked. While
that read is pending or after it failed, the page says "Source coverage
unknown" and also marks the figures provisional: an unread coverage is not a
current one. Exports carry the same verdict: the full report (period end: its
generation time, scoped to its wallet), the summary (its timeframe end and
wallets) and the Austrian report (its tax year's end, or today for the running
year) add a provisional note and a table of the lagging sources with their
action to the issues section of the PDF and XLSX, `source_coverage.csv` plus a
`source_coverage,notice` row in `summary.csv`, and `source_coverage_notice` in
the CSV manifest. Export proceeds, no figure changes, and a book whose sources
all reach the period end exports exactly as before.
`ui.report.blockers` is unchanged: recorded sessions and the frozen
Python-derived goldens pin its payload, and it takes no period. The assistant
reads `ui_sources_coverage` with the report's period end instead
([AI](ai.md)).

## Loading and freshness

Screens show skeletons only before their first data arrives. Once a screen has
shown data, it keeps showing it until the new data replaces it; a refresh
never blanks it. The one exception is a different dataset: picking another
tax year on Reports shows a placeholder until that year's report arrives,
because the previous year's figures under the new heading would be wrong.
Filter and period changes on Transactions keep the previous rows dimmed while
the new ones load.

A sync, a journal run, a market-rate refresh, a background freshness event,
or a mutation that changes the book starts a book refresh cycle
(`refresh_reads` in `src/app.rs`). Every screen keeps the data it showed while
the cycle re-reads. The cycle reads the book's
revision (`status` with `revision: true`, see
[request routing](daemon.md#request-routing)) before and after the refetch, and
refetches again when a write landed in between, up to three passes. Then all
screens switch to the new data in the same render, so the Overview balance
and its transaction list always come from one state of the book. Each cycle
holds the screen for at most 15 seconds; requests made during a cycle merge
into one follow-up cycle. The daemon refuses reads while a user-started sync
runs, so a cycle never shows the book halfway through one; background
workers can commit between cycles, which the next cycle picks up. While a
cycle runs, the re-reads' answers are held until the last one arrives.

The title bar's book status names one state for the whole book: updating
(syncing wallets, processing journals, or loading changes), refresh needs
attention, journals need processing, or up to date. After a cycle it names
what changed for 30 seconds (new or removed transactions and the balance
move, masked in privacy mode), and rows the cycle brought in are highlighted
on Transactions and the Overview's latest transactions for a few seconds. It
is the screens' one live region for refreshes, kept for screen readers on
narrow windows where it is not shown. While the book updates, screens dim
only the figures that will change; they stay readable and clickable, and the
table pauses loading more rows until the update lands. Reads stay cached by
kind, so reopening a screen shows its data at once while it re-reads. Every
lock, book switch, and profile switch empties the cache and forgets the change
summary.

# Kassiber desktop

Kassiber's desktop app: a native Rust UI built with [Rune](https://github.com/anomalyco/rune),
Anomaly's GPU-rendered UI framework. It runs the bundled `kassiber` binary as its daemon and
speaks the same JSONL protocol the CLI's other front ends use, so every accounting decision
stays in the backend crates. The UI is in English and Austrian German.

This crate is its own Cargo workspace with its own `Cargo.lock`. The root workspace excludes it,
so Rune and its graphics stack never enter the backend's dependency tree. Rune is pinned by git
revision (`Cargo.toml`), with the `[patch.crates-io]` entries Rune asks apps to copy. It builds
with the repository's toolchain (`rust-toolchain.toml` at the root).

Read from the repository root:

| Path | What |
| --- | --- |
| `assets/i18n/locales/{en,de}/*.json` | the i18next bundles, embedded at build time (`src/i18n.rs`) |
| `assets/logos/integrations/` | connection and rail logos (`src/logos.rs`, `src/overview/catalog.rs`) |

The fonts (`assets/fonts/`, DM Sans and JetBrains Mono) and the app icons
(`packaging/icons/`) are this crate's own.

## Run it

```bash
cargo build --release -p kassiber              # from the repository root: the daemon
cp -a <a disposable book>/ /tmp/rune-try/       # see docs/reference/testing.md
crates/kassiber-desktop/scripts/run.sh --data-root /tmp/rune-try/data
```

`scripts/run.sh` builds the app (`cargo build --release`, with agent tooling) and starts it on
that book, with every preference file in a scratch directory
(`$KASSIBER_DESKTOP_SCRATCH_PREFS`, default `/tmp/kassiber-desktop-prefs`), so the account's
own Kassiber state root is neither read nor written. The daemon opens the book you pass and its
buttons act on it: use a copy. Turn offline mode on (`kassiber offline on` with the same
preference file) or set `KASSIBER_NO_EGRESS=1` when you want no network at all.

On launch the app makes only local reads. Network actions (book refresh, rate refresh,
connection tests, graph lookups) run when you press them, or under the narrowly scoped
exceptions in [privacy and security](../../docs/reference/privacy-and-security.md).

Linux builds need `pkg-config`, fontconfig and the Wayland client headers
(`libfontconfig1-dev libwayland-dev`), and Vulkan at run time.

## Checks

```bash
packaging/ci/rune-checks.sh --skip-scenes                     # fmt, clippy (both feature sets), tests
packaging/ci/rune-checks.sh --kassiber-bin ../../target/debug/kassiber   # + the scene check
```

- `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings` with default features
  (agent tooling) and with `--no-default-features` (the shipped configuration).
- `cargo test --release`: the unit tests. They use disposable roots and make no network calls. The redaction test reads the backend's scanner corpus, locked test data that `scripts/fetch-test-data.sh` (repository root) puts in place.
- The scene check (`src/checks.rs`, `#[ignore]`d in plain test runs) renders every scene
  headless against a disposable book and fails on accessibility-tree problems and missing
  translations. It runs offline with `KASSIBER_NO_EGRESS=1` in a temporary `HOME`.

`scripts/quality-gate.sh` runs the first form; CI's "Desktop app" job runs it on macOS, Linux
and Windows. The scene check is not in CI yet: on an empty book every Transactions scene waits
out its 30 s load deadline, so a full run takes hours (see `TODO.md`). `RUNE_CHECK_SCENES=a,b`
limits it to named scenes, and each render is logged as `scene check: <name> (<lang>)`.

## Headless tooling

Builds with the `tooling` feature (the default, and every debug build) take Rune's agent
arguments; shipped builds (`--no-default-features`) refuse them.

```bash
bin=target/release/kassiber-desktop
$bin --data-root … --snap out.png --size 1440x900 --scene transactions
$bin --data-root … --tree tree.json --scene detail-tax          # the accessibility tree
$bin --data-root … --script scripts/txs-sort.json --out frames/ --trace run.jsonl
$bin --data-root … --lang de --snap de.png --scene transactions
```

- **Scenes** are named states for snapshots and the parity harness. Each area lists its own in
  a `SCENES` table (`crate::app::Scene`); `all_scenes` in `src/main.rs` collects them. Scenes
  marked `SYNTHETIC` inject sample state the demo book can't reach; nothing they inject is
  written to the book unless the scene name says so.
- **Scripts** (`scripts/*.json`, `scripts/a11y/`) are recorded input sequences: clicks, keys,
  waits and captures.
- **Fixtures** (`parity/fixtures.json`, read by `src/fixtures.rs`) name the demo-book rows
  scenes use, so a scene asks for `{fx:full}` instead of hardcoding an id, and define named
  book variants that a scene builds on a temporary copy of the book (`fixtures::use_book`).
  `tools/demo-fixtures.py resolve <book>` rewrites the ids for a new demo book.

## Parity harness

`parity/` drove this app and the removed Tauri app into the same state on identical copies of
the regtest demo book, captured each scene and diffed pixels, text layout, controls and
containers. It still runs against a separate checkout of the last commit with `ui-tauri/`
(`PARITY_TAURI_CHECKOUT`); setup and usage are in [parity/README.md](parity/README.md), the
last measured results in [docs/parity.md](docs/parity.md).

## Packaging

| Script | Output |
| --- | --- |
| `packaging/macos/build-app.sh` | `Kassiber.app` (arm64), the shape `kassiber-release macos sign/notarize/verify` takes |
| `packaging/linux/build-packages.sh` | the `kassiber` .deb and an AppImage |
| `packaging/windows/build-installers.sh` | the MSI and NSIS installers |

Each installs the app, built with `--no-default-features`, as `kassiber-ui` beside the
`kassiber` sidecar and terminal launcher; the macOS and Windows scripts also check that the
binary refuses headless arguments. `packaging/licenses/inventory.sh` lists every locked package with its licence from
local data only. See [Rust packaging](../../docs/reference/rust-packaging.md) and
[macOS release](../../docs/reference/macos-release.md).

## Layout

| Path | What |
| --- | --- |
| `src/main.rs`, `src/app.rs` | entry point, fonts, scene table; app state, request tags, cached reads, navigation |
| `src/daemon.rs`, `src/kinds.rs` | the daemon child process and its JSONL protocol; the desktop kind allowlist |
| `src/i18n.rs`, `src/format.rs`, `src/theme.rs` | i18next subset over the shared bundles; `Intl`-compatible formatting; design tokens |
| `src/shell/`, `src/ui/`, `src/ui.rs` | sidebar, title bar, search, popovers, overlays; shared widgets |
| `src/overview/`, `src/chart/` | Overview and the BTC activity chart |
| `src/transactions/`, `src/txdetail/` | Transactions workbench; the transaction detail panel and graph |
| `src/wallets/`, `src/review/`, `src/custody/`, `src/provenance/`, `src/reports/` | the other screens |
| `src/settings/`, `src/assistant/`, `src/onboarding/` | Settings, the AI assistant, first run and unlock |
| `src/native/`, `src/secrets/`, `src/osleaks/` | OS integration: deep links, notifications, CLI forwarding, updates, credential store, backup exclusion |
| `src/checks.rs` | the scene accessibility and translation check |
| `tools/` | developer tools: the icon generator, logo bounds, click-through and glass checks (stdlib `python3 -I` or Node; not part of the gate) |
| `docs/` | parity results; `docs/history/` keeps the November 2026 evaluation records |

# Kassiber native macOS client

`ui-macos` is a SwiftUI/AppKit peer of the Tauri and CLI clients. It owns presentation and local sidecar supervision; the Python daemon and SQLite/core layers remain the source of truth.

## Requirements

- macOS 15 or newer (Textual 0.5 requirement)
- Xcode with Swift 6.1+
- The repository Python environment (`../.venv`) or `uv`

## Build and test

```sh
cd ui-macos
python3 Scripts/generate_daemon_kinds.py --check
python3 Scripts/sync_string_catalog.py --check
swift test
swift run kassiber_native
```

The generated daemon-kind contract must be refreshed after `SUPPORTED_KINDS` changes:

```sh
python3 Scripts/generate_daemon_kinds.py
```

On Linux, the manifest omits the SwiftUI/AppKit, Textual, and Sparkle target. `swift test` therefore builds and tests the Foundation/Observation protocol and view-model subset. The screenshot and app-launch checks are macOS-only.

## Development sidecar

The app starts one daemon and speaks JSONL on stdin/stdout. Development launch resolution is:

1. `KASSIBER_PYTHON`, when set;
2. `../.venv/bin/python -m kassiber ... daemon`;
3. `/usr/bin/env uv run python -m kassiber ... daemon`.

Useful overrides:

```sh
KASSIBER_DATA_ROOT=/path/to/data \
KASSIBER_REPO_ROOT=/path/to/kassiber \
swift run kassiber_native
```

The supervisor demultiplexes by `request_id`, preserves fragmented JSONL ordering, accepts interleaved streaming records, and routes only `event: true` records without request ids as unsolicited events. Calls are deny-by-default against the exact Tauri renderer allowlist, not merely the larger generated `SUPPORTED_KINDS` enum. AI runtime kinds have a separate fail-closed toggle that cancels in-flight chat when disabled. Stderr and supervisor lifecycle records are secret-redacted, bounded, and kept only in memory.

The packaged app registers the same fixed `kassiber://` navigation, settings, workflow, and lock deep-link contract as Tauri. Native menus expose the matching navigation shortcuts, global refresh/reprocess workflows, sensitive-value and UI-scale controls, settings sections, documentation, and issue reporting. `⌘K` searches localized pages, actions, connections, recent transactions, and exact local transaction identifiers without crossing into AI-only search kinds. The full settings workstation is available both in the app navigation and the standard macOS Settings scene.

## Localization

Edit both `Sources/KassiberApp/Resources/en.lproj/Localizable.strings` and `de.lproj/Localizable.strings`, then run:

```sh
python3 Scripts/sync_string_catalog.py
```

The localization test checks both `.strings` files and `Localizable.xcstrings` remain in lockstep. German is Austrian and uses informal “du”.

## Local arm64 app bundle

`scripts/build-mac-arm64-native-app.sh` is the public entry point for the
separate native build. It delegates to `ui-macos/Scripts/build-macos-arm64-app.sh`.
The existing `scripts/build-macos-arm64-app.sh` remains the Tauri/React web
frontend build and is not replaced. On an Apple Silicon Mac the native build:

- checks generated daemon/localization contracts and runs the Swift tests;
- invalidates older output bundles before any build work, so a failed pass
  cannot leave a stale app or ZIP looking current;
- builds (or accepts) a self-contained arm64 Python sidecar;
- builds the release SwiftPM product and assembles `build/kassiber_native.app`;
- normalizes SwiftPM's generated command-line resource accessors for a signed
  macOS bundle, then embeds every resource bundle under `Contents/Resources`
  and an arm64-thinned `Sparkle.framework` under `Contents/Frameworks`;
- runs the bundled Kraken daemon smoke against both the source and signed
  sidecar and verifies every embedded Mach-O is arm64-only;
- uses `ui-tauri/src-tauri/icons/icon.icns` directly and verifies the bundled
  runtime PNG is a byte-for-byte copy of Tauri's `icon.png`;
- embeds Kassiber's AGPL-3.0 license plus complete direct/transitive native
  dependency notices under `Contents/Resources`;
- applies a launchable ad-hoc local signature and emits
  `build/kassiber_native-macos-arm64.zip`.

Run the complete local build with:

```sh
./scripts/build-mac-arm64-native-app.sh
```

To reuse an existing self-contained arm64 sidecar (and skip PyInstaller):

```sh
KASSIBER_SIDECAR_SOURCE=/absolute/path/to/kassiber-cli \
  ./scripts/build-mac-arm64-native-app.sh
```

The real daemon/bundled-Kraken smoke always runs before and after signing,
including when `KASSIBER_VERIFIED_SIDECAR_SHA256` supplies an additional exact
binary pin; the hash never bypasses runtime verification. A mismatch fails the
build. `SWIFTPM_DISABLE_SANDBOX=1` disables SwiftPM's nested sandbox for both
tests and release compilation when a stricter outer sandbox already enforces
filesystem boundaries.

`SKIP_TESTS=1` is available for a repeat packaging pass after the same checkout
already passed tests. `CREATE_ZIP=0` leaves only the `.app`. The product,
executable, display name, and app bundle are all named `kassiber_native`; its
default bundle identifier is `at.bitcoinaustria.kassiber.native` and can be
overridden with `BUNDLE_IDENTIFIER`.

## Developer ID distribution

`Scripts/build-distribution.sh` is documented automation and is not run by tests. It requires:

- `DEVELOPER_ID_APPLICATION`
- `APPLE_TEAM_ID`
- `NOTARY_PROFILE` created with `xcrun notarytool store-credentials`
- `KASSIBER_SIDECAR_SOURCE`, a self-contained signed Kassiber executable accepting global flags followed by `daemon`

The distribution wrapper delegates assembly to the same local script, signs
the sidecar, Sparkle framework, executable, and `kassiber_native.app` from the
inside out with hardened runtime and timestamping, submits the ZIP to
`notarytool`, staples and validates the ticket, verifies it with Gatekeeper,
then recreates the archive so it contains the stapled app. App Store
distribution is intentionally unsupported because Kassiber is AGPL.

Sparkle 2.9.4 is scaffolded but dormant. A release bundle must add `SUFeedURL` and `SUPublicEDKey` to `Info.plist`; until both exist, the update controller is not created and “Check for Updates” is disabled.

`Sources/KassiberApp/Resources/AppIcon-1024.png` is intentionally a byte-for-byte
mirror of Tauri's `ui-tauri/src-tauri/icons/icon.png` for `swift run` launches.
Packaged builds copy Tauri's already-generated `icon.icns` directly, so the
native and Tauri products use the exact same icon assets rather than two
separately generated approximations.

## Verification

Live screenshots are under `verification/`. They use the repo's unencrypted
regtest demo book; no screen-wide capture is used. The capture script keeps one
packaged app process per language, navigates the native routes, and renders a
fresh AppKit frame from that exact process's own window after each route
settles. Verification is pinned to Aqua for deterministic native materials and
crops the NavigationSplitView's separately composited sidebar column; the
route detail, titlebar, toolbar, stock controls, charts, tables, and live daemon
data are the actual app view hierarchy. No desktop or other application's
pixels are read, and the optional ScreenCaptureKit path remains available for
interactive runs. The app writes a per-frame receipt containing the route,
locale, dimensions, PNG digest, executable digest, capture backend, and build
identity only after the native render succeeds. The validator checks every PNG
chunk CRC, fully decodes RGB/RGBA scanlines, composites alpha, and rejects
missing, undersized, corrupt, transparent, duplicate, blank, or near-uniform
frames. A successful full run writes the schema-v2
`verification/manifest.json`; it verifies all 24 receipts, the expected product
and bundle ID, the deep code signature, every Mach-O as arm64-only, exact Tauri
icon assets, required resources, a deterministic hash of the entire app tree,
and that the ZIP extracts to that same tree. Every new run invalidates the old
manifest first; partial/resumed mixed-build evidence is not accepted.

Normal Finder and `open` launches continue to reuse the running application.
The verifier alone launches the packaged Mach-O as a direct child with an exact
PID and terminates only that capture process. This lets verification run while
a user's existing Kassiber window stays open, without changing process-global
or launchd environment state. Capture-only preferences live under an isolated
temporary home, so a user's Touch ID, lock, appearance, or onboarding defaults
cannot make the evidence run nondeterministic.

See [PARITY.md](PARITY.md) for the source-derived completion ledger.
`Scripts/check_frontend_parity.py` fails when any of the 22 Tauri routes lacks
an explicit native screen/surface mapping, or when one of its 384 route/kind
memberships is absent from that route's named Swift declaration owners. This
route-scoped audit cannot be satisfied by a callsite on an unrelated screen.
It also locks Activity, transfer-editing, New Transaction, transaction-detail
graph/privacy, and Assistant actions to their SwiftUI presentation and
view-model calls. The audit runs inside `swift test` alongside scripted-daemon
behavior tests.

No Python daemon additions were needed.

## Known release constraints

- The native target requires macOS 15 because Textual 0.5 uses macOS 15 attributed-text APIs.
- Sparkle intentionally performs no network request until a signed feed URL and Ed25519 public key are supplied.
- Developer ID signing/notarization is fully scripted but cannot be executed without the release team's Apple identity and stored `notarytool` profile; the local script produces a launchable ad-hoc-signed arm64 bundle and ZIP.
- App Store distribution is intentionally unsupported because Kassiber is AGPL-3.0-only.

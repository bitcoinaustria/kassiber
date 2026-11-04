# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber currently depends directly on [BDK Python](https://github.com/bitcoindevkit/bdk-python) for dependency-backed Bitcoin observation, [Liquid Wallet Kit](https://github.com/Blockstream/lwk) for dependency-backed Liquid observation, [embit](https://github.com/diybitcoinhardware/embit) for compatibility descriptor derivation and Liquid decoding, [XlsxWriter](https://xlsxwriter.readthedocs.io/) for styled XLSX report exports and the generic-ledger import template, [openpyxl](https://openpyxl.readthedocs.io/) for reading filled-in `.xlsx` generic-ledger imports, [ReportLab](https://www.reportlab.com/) for styled PDF report exports, and [keyring](https://github.com/jaraco/keyring) for opt-in CLI access to native OS credential stores.
The descriptor connection screen uses local-only QR scanner libraries for
webcam-based descriptor and BBQR import.
The desktop shell includes Rust keyring crates for AI-provider-key native
storage. This applies only to AI provider API keys; backend tokens, descriptors,
xpubs, blinding keys, and reveal payloads remain SQLCipher-backed.
The R1 Rust runtime reuses those explicit native stores for bound CLI
remembered unlock, with Python-compatible items in its separate namespace.
Its minimal release notifier delegates the GitHub request to the bundled CLI
sidecar rather than carrying a second HTTP client. The desktop and CLI
coordinate update-check consent revocation with an owner-only advisory lock;
the native side uses `fs2` for the cross-platform file-lock primitive.
Kassiber's own tax engine is a Rust crate (`tax-engine/`, AGPL-3.0-only) built
as a Python extension; the rows below list its direct Rust dependencies, and
the root `Cargo.lock` is the transitive inventory.
The private custody fusion probe reuses this existing tax-engine crate through
a development dependency; this adds no external package or runtime dependency.
The native journal pre-build reuses Kassiber's transfer crate for component
normalization, revisions, activation and evidence commitments. This internal
dependency adds no third-party package.
The Kassiber-maintained [RP2 fork](https://github.com/bitcoinaustria/rp2), which the engine replaced,
is a development dependency only: tests use it as the engine's parity oracle.
Rust storage also uses the existing internal `kassiber-pydecimal` crate for
legacy Liquid fingerprint arithmetic and native file-import amount/pricing normalization; this adds no third-party package.
Historical report reads reuse the internal `kassiber-wallets` resolver to
preserve profile scoping and ambiguous-label refusals. This adds no external
package or license; existing descriptor-library versions remain unchanged.
Transfer authoring reuses it for exact BTC-to-msat conversion and the existing
locked `bitcoin_hashes` SHA-1 implementation for Python-compatible UUIDv5 review
identities; the standard library has no SHA-1 implementation. Canonical
commitments reuse the internal custody and `kassiber-pyjson` crates.
Journal persistence reuses the same locked `bitcoin_hashes` SHA-1 primitive
for filed-impact UUIDv5 identities, the store's bundled SQLCipher driver,
and the internal rates timestamp parser. No package version changes.
The unregistered transfer review harness links the existing internal journal
assembler directly to the case reader. This adds no external package or license.
Review operation digests reuse the same locked `bitcoin_hashes` SHA-256 primitive.

> Inherited from `t3code/r4-12-persist-accept@7536e8214`; pending r10 verification.
Persistence tests enable the existing store `test-hooks` feature's rusqlite
authorizer to inject real PRAGMA refusals through its safe API. The standard
library cannot inject SQLite authorization errors; no package or lockfile
version changes are needed, and ordinary builds do not enable these hooks.
The Rust setup services reuse the internal store, envelope, and Python-compatible
JSON/text crates for existing book formats. Network partitions add `age` for
the existing passphrase-encrypted backup format and `tar` for its archive;
the standard library and existing workspace code provide neither format.
The Rust backup service reuses those same locked `age` and `tar` crates for
Python-compatible `.kassiber` archives. In-process passphrase encryption avoids
passing secrets to a child process; recipient mode retains Python's fixed
`age`/`rage` invocation. No registry version or checksum changes.
Desktop backup previews also reuse the locked `sha2` 0.11 staging digest and
`getrandom` 0.4.3 preview capabilities; the standard library provides neither
SHA-256 nor a portable cryptographic random-byte API.
They reuse SHA-256, UUID parsing, temporary-file publication and the existing
lookaround regex engine for Python's public-watch-material projection.
Machine preferences also reuse `tempfile` in storage at runtime to atomically
publish private files; the standard library has no private temporary-file
allocator with automatic cleanup.
The Rust secrets lifecycle reuses `tempfile` for private credential-migration
staging and `zeroize` for transport copies. It directly uses the already locked
`getrandom` version for credential enrollment and invalidation generations;
the standard library has no portable cryptographic random-byte API.
Report CSV publication shares that storage helper. CLI publication tests reuse
the same pinned `tempfile` crate for disposable directories and cleanup; no
third-party package or runtime requirement is added.

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `getrandom` | `0.4.3` (exact) | OS randomness for Rust credential enrollment, invalidation generations and backup preview capabilities | MIT OR Apache-2.0 |
| `bdkpython` | `3.0.0` (exact) | Watch-only Bitcoin wallet observation through BDK's Electrum client; state is persisted as explicit versioned JSON inside Kassiber's SQLCipher database. Esplora uses Kassiber's compatibility HTTP transport (`http_route_policy`) | MIT OR Apache-2.0 |
| `bdk_wallet` | `=3.0.0` | Native in-memory watch-only wallet state and indexing matching bdkpython; standard library and descriptor parsing alone cannot reproduce BDK's update model. No BDK storage or connector features | MIT OR Apache-2.0 |
| `bdk_chain` | `=0.23.3` | Native graph, chain checkpoints and changeset merging matching the Python binding; fetched data comes through Kassiber's consent-bound clients | MIT OR Apache-2.0 |
| `bdk_core` | `=0.6.3` | Exact BDK script requests, transaction updates and confirmation anchors; keeps the native observer's update contract pinned to Python | MIT OR Apache-2.0 |
| `ahash` | `0.8.12` (transitive) | BDK's pinned graph/index hash tables require this hasher; selected by the matching SDK rather than a new application API | MIT OR Apache-2.0 |
| `hashbrown` | `0.14.5` (transitive) | BDK's pinned chain/index implementation requires these hash tables; replacing them would fork the reference SDK | MIT OR Apache-2.0 |
| `base64` | `0.21.7` (transitive) | BDK's pinned core serialization dependency; kept alongside existing 0.22 to preserve the matching SDK without patching it | MIT OR Apache-2.0 |
| `lwk` | `0.18.0` (exact) | Watch-only Liquid wallet observation through LWK's Electrum client; opaque `ForeignStore` values live only inside Kassiber's SQLCipher database. Esplora uses Kassiber's compatibility HTTP transport (`http_route_policy`) | MIT (BSD-MIT wording) |
| `embit` | `>=0.8.0` | Bitcoin/Liquid descriptor parsing, script derivation, Liquid confidential outputs, PSBT transaction/UTXO parsing and supported Payjoin receiver signature commitments | MIT |
| `bitcoin_hashes` | `0.14` (already locked) | Python-compatible UUIDv5 review identities using the existing SHA-1 primitive | CC0-1.0 |
| `bitcoin` | `0.32` | Native watch-only BIP32 derivation, key validation, scripts and addresses; cryptography and Bitcoin encodings have no standard-library equivalent | CC0-1.0 |
| `miniscript` | `12` | Native Bitcoin descriptor grammar, multipath expansion and script derivation; avoids a handwritten script interpreter | CC0-1.0 |
| `elements-miniscript` / `elements` | `0.4` / locked transitive `0.25` | Liquid descriptor grammar, SLIP-77 blinding and confidential address derivation; avoids reimplementing confidential address cryptography | CC0-1.0 |
| `secp256k1` / `secp256k1-zkp` | locked transitive | Bitcoin and Liquid public-key operations used by the descriptor libraries; no signing or broadcast API is exposed by Kassiber | CC0-1.0 |
| `regex` | `1` | Bounded non-backtracking descriptor token and wallet-export syntax normalization; already used transitively by backend redaction | MIT OR Apache-2.0 |
| `XlsxWriter` | `>=3.2,<4` | Styled `.xlsx` workbook export for practitioner-facing reports and the generic-ledger import template | BSD-2-Clause |
| `rust_xlsxwriter` | `0.102.0` (exact, optional features off) | In-memory native XLSX reports, styles and cached formulas; OOXML rendering is unavailable in the standard library | MIT OR Apache-2.0 |
| `zip` | `9.1.3` (exact, default features off, `deflate`) | Writer's ZIP container and in-memory repair of point heights/numeric XML; no archive extraction or new transport | MIT |
| `zlib-rs` / `zopfli` | locked transitive `0.6.8` / `0.8.4` | Writer's pure Rust DEFLATE backends; no system zlib dependency | Zlib / Apache-2.0 |
| `typed-path` / `nix` | locked transitive `0.13.0` / `0.31.3` | ZIP path types and platform support; renderer uses only RAM buffers | MIT OR Apache-2.0 / MIT |
| `crc32fast` / `simd-adler32` | locked transitive `1.5.3` / `0.3.10` | ZIP/compression checksums; updates required by the cached Zopfli dependency | MIT OR Apache-2.0 / MIT |
| `openpyxl` | `>=3.1,<4` | Reads filled-in `.xlsx` files for the generic-ledger manual importer | MIT |
| `reportlab` | `>=4.4,<5` | Styled PDF rendering for Austrian and source-of-funds report exports | BSD |
| `sqlcipher3` | `>=0.6.2,<1` | Python binding around SQLCipher 4; wheels bundle a SQLCipher community build for at-rest database encryption | Zlib (binding) + BSD-style (SQLCipher community) |
| `pyrage` | `>=1.3,<2` | In-process `age` implementation used by the `tar | age` backup format when no system `age`/`rage` binary is available | Apache-2.0 / MIT |
| `keyring` | `>=25.6,<26` | Native macOS Keychain, Windows Credential Manager, and Linux Secret Service access for opt-in CLI remembered unlock | MIT |
| `tzdata` | `>=2024.1` | Time zone data for `zoneinfo` where the OS has none, such as Windows | Apache-2.0 |
| `rp2` | `git+https://github.com/bitcoinaustria/rp2.git@3d2e03ef27696fbd00f4516a0cb745300083e227` (dev only) | Parity oracle for Kassiber's tax engine: the frozen phase-3 adapter in `tests/tax_oracle/` runs on it, tests compare the engine with that oracle, and `tax-engine/scripts/gen_engine_fixtures.py` and `gen_ledger_fixtures.py` build fixtures from it; not shipped | Apache-2.0 |
| `cryptography` | `>=50.0.0,<51` (dev only) | Self-signed TLS certificates for the regtest chain-observer oracle test; the application does not import it | Apache-2.0 OR BSD-3-Clause |
| `PyYAML` | `>=6,<7` | Structured parsing for the repository-wide GitHub Actions pin-policy test | MIT |
| `pako` | `^2.1.0` | Zlib/deflate decoding for Better Bitcoin QR descriptor import | MIT |
| `qr-scanner` | `^1.4.2` | Local webcam QR decoding for descriptor-family connection setup | MIT |
| `three` | `0.186.0` (exact) | Glass ribbon 3D view of a transaction graph, loaded only when that view opens; every mesh, material and light is generated locally, with no models, textures or network requests. `@types/three` supplies typings only and is not bundled | MIT |
| `react-markdown` | `^10.1.0` | Markdown renderer for assistant chat replies (paragraphs, lists, code, links) | MIT |
| `remark-gfm` | `^4.0.0` | GitHub-flavored markdown extensions (tables, strikethrough, task lists) for assistant chat replies | MIT |
| `i18next` | `25.8.18` (exact) | Desktop UI localization runtime (English/German, expandable); see [docs/reference/i18n.md](docs/reference/i18n.md) | MIT |
| `react-i18next` | `16.5.8` (exact) | React bindings (hooks/provider) for i18next translations | MIT |
| `@opencode-ai/sdk` | `1.18.10` (exact) | Typed client for the loopback OpenCode server supervised by Kassiber | MIT |
| `@modelcontextprotocol/sdk` | `^1.30.0` | Ephemeral local MCP server that carries Kassiber's typed tool schemas into the Claude, OpenCode, and Copilot CLIs | MIT |
| `@agentclientprotocol/sdk` | `1.5.0` (exact) | Agent Client Protocol client for the GitHub Copilot CLI supervised by the chat broker | Apache-2.0 |
| `zod` | `4.4.3` (exact) | JSON-Schema-to-validator conversion for the tool schemas that MCP server advertises | MIT |
| `fs2` | `0.4.3` | Cross-platform exclusive advisory lock for update-check consent and in-flight GitHub requests | MIT OR Apache-2.0 |
| `dirs` | `6.0.0` | Resolves an absolute per-user home directory before the Tauri app handle exists | MIT OR Apache-2.0 |
| `keyring-core` | `1.0.0` | Rust trait layer for desktop AI-provider storage and runtime CLI remembered unlock; no standard-library native credential API | MIT OR Apache-2.0 |
| `apple-native-keyring-store` | `1.0.0` | macOS Keychain backend for AI provider API keys and opt-in database passphrase remember-unlock | MIT OR Apache-2.0 |
| `block2` | `0.6.2` | Objective-C block bridge for the macOS Touch ID LocalAuthentication callback | MIT |
| `objc2` / `objc2-foundation` / `objc2-app-kit` | `0.6.4` / `0.3.2` / `0.3.2` | Rust Objective-C bridge used for macOS Touch ID prompts and AppKit's main-window frame autosave | MIT |
| `tauri-plugin-window-state` | `~2.4` (Windows and Linux only) | Remembers the main window's size, position, and maximized state across launches, restoring a position only onto a connected monitor. macOS uses AppKit's frame autosave instead | Apache-2.0 OR MIT |
| `windows-native-keyring-store` | `1.0.0` | Windows user-scope credential backend for AI provider API keys and Python-compatible CLI remembered unlock | MIT OR Apache-2.0 |
| `zbus-secret-service-keyring-store` | `1.0.0` | Explicit Linux Secret Service backend for AI provider API keys and Python-compatible CLI remembered unlock; runtime use is isolated behind `kassiber-secrets/native-keyring`, keeping D-Bus/Tokio out of core-only builds; avoids kernel-keyutils fallback | MIT OR Apache-2.0 |
| `pyo3` | `0.25.1` | Python binding for the tax engine (abi3, CPython 3.10 and newer); 0.24.1 or newer is required for RUSTSEC-2025-0020 | MIT OR Apache-2.0 |
| `num-bigint` / `num-integer` / `num-traits` | `0.4` / `0.1` / `0.2` | Arbitrary-precision coefficients for the CPython-compatible decimal type (`crates/pydecimal`) the tax engine and the backend crates share, and Python's unbounded ints in `crates/pyjson` | MIT OR Apache-2.0 |
| `serde` / `serde_json` | `1.0.228` / `1` | JSON request/response documents and storage manifests at the tax and custody Python boundary; wallet CLI config intake reuses the locked arbitrary-precision parser because the standard library has no JSON parser; persisted Python text uses `kassiber-pyjson`; custody enables arbitrary integers and correctly rounded float parsing; its CPython codec preserves raw object order; the existing `raw_value` feature retains unported local snapshot fragments during the typed migration, with no new package or lockfile dependency | MIT OR Apache-2.0 |
| `fancy-regex` | `0.20` | Backtracking regexes (over `regex-automata`/`regex-syntax`) for redaction and partition public-watch-material projections, which need lookbehind and lookahead; local attachment filename sanitization reuses the same frozen CPython Unicode class | MIT |
| `age` | `0.12` | In-process streaming passphrase encryption for partition and `.kassiber` backups, compatible with Python's pyrage format | MIT OR Apache-2.0 |
| `tar` | `0.4` | Partition and `.kassiber` archive encoding/reading; the latter retains Python's count-only attachment manifest | MIT OR Apache-2.0 |
| `rustls` / `rustls-webpki` | `0.23` / locked transitive | TLS and certificate/hostname verification over `kassiber-net`'s guarded stream; the standard library has no TLS implementation. Automatic networking is never delegated to a client SDK | Apache-2.0 OR ISC OR MIT / ISC |
| `ring` / `untrusted` | locked transitive | rustls's explicitly selected cryptographic provider and bounded cryptographic input parsing; no new system TLS library | Apache-2.0 AND ISC / ISC |
| `rustls-native-certs` / `rustls-pki-types` | `0.8` / locked transitive | Platform root selection and PEM/DER parsing for immutable consent-bound trust snapshots; unavailable in the standard library | Apache-2.0 OR ISC OR MIT / Apache-2.0 OR MIT |
| `webpki-roots` | `1` | Embedded Mozilla CA fallback matching the frozen Python certifi bootstrap's intent, without downloading roots at runtime | CDLA-Permissive-2.0 |
| `httparse` | `1.10` | Mature bounded HTTP/1.1 header parser; a full HTTP client could introduce an ungated connector | MIT OR Apache-2.0 |
| `url` / `idna` / `idna_adapter` | `2.5` / locked transitive / `~1.0`, default features off | URL and redirect-origin parsing without DNS; the adapter's ASCII-only mode enforces reviewed A-label host identities without pulling in ICU4X | MIT OR Apache-2.0 |
| `form_urlencoded` / `percent-encoding` / `utf8_iter` / `smallvec` | locked transitive | URL query/escape parsing and IDNA ASCII validation support | MIT OR Apache-2.0 |
| `base64` | `0.22` | Explicit HTTP proxy Basic authentication, separately from target headers; reuses the desktop dependency | MIT OR Apache-2.0 |

| `rusqlite` / `libsqlite3-sys` | `0.40.1` (exact) / locked transitive | Existing-book Rust runtime; SQLite's backup/transaction APIs and bundled SQLCipher are unavailable in the standard library | MIT / MIT (bindings); public domain (SQLite), BSD-style (SQLCipher community) |
| `openssl-src` / `openssl-sys` | locked transitive | Vendored SQLCipher crypto on Linux/Windows avoids an undeclared system OpenSSL runtime; macOS selects CommonCrypto | Apache-2.0 / MIT |
| `sha2` | `0.10` (custody), `0.11` (storage, attachments and backup staging) | Persisted custody evidence/observation commitments, Python-compatible project/schema/remembered-account digests, managed attachment hashes and desktop restore staging commitments; unified with native Secret Service's RustCrypto generation; no standard-library cryptographic hash | MIT OR Apache-2.0 |
| `ssh-key` / `signature` | `=0.6.7` / `=2.2.0`, default features off, `std` and `ed25519` only | In-process `verify-download` signature check replacing the `ssh-keygen -Y verify` subprocess: the SSHSIG signed-data encoding and Ed25519 verification. Kassiber keeps OpenSSH's own armour and field parsing, which the crate's stricter PEM decoder does not reproduce. No key generation, encryption, RSA, ECDSA or I/O | Apache-2.0 OR MIT |
| `ed25519-dalek` / `ed25519` / `ssh-encoding` / `ssh-cipher` / `pem-rfc7468` / `base64ct` | locked transitive (`2.2.0` / `2.2.3` / `0.2.0` / `0.2.0` / `0.7.0` / `1.8.3`) | ssh-key's Ed25519 verifier (canonical-scalar checking, as OpenSSH 10.6 verifies) and its wire, cipher-name and PEM support; the lockfile also lists ssh-key's optional RSA/ECDSA crates, which are not compiled | BSD-3-Clause / Apache-2.0 OR MIT |
| `zeroize` | `1` | Erases application-owned passphrase, fd input and key-literal buffers on drop, including error paths | MIT OR Apache-2.0 |
| `nix` / `libc` | `0.30` / `0.2` | OS-account-home/uid lookup, no-follow file flags, exclusive atomic storage publication unavailable in the standard library, consuming raw passphrase fds without assuming a valid Rust descriptor, and test-only journal-loader process CPU timing | MIT / MIT OR Apache-2.0 |
| `windows-sys` | `0.61` | Windows native ownership SID checks, stable AppData, and Python-compatible file identity; no standard-library owner/SID API | MIT OR Apache-2.0 |
| `rpassword` | `7` | Restoring no-echo controlling-terminal passphrase prompt; the standard library has no portable terminal echo control | Apache-2.0 OR MIT |
| `toml_edit` | `0.25`, parse only (dev only) | Parse Cargo dependency, feature, patch/replace and source policy in the network chokepoint regression scanner; the standard library has no TOML parser, and line matching missed valid Cargo syntax | MIT OR Apache-2.0 |
| `tempfile` | `3` | Private partition staging and atomic publication of exports and machine preferences; disposable Rust unit-test books, metadata and ownership namespaces | MIT OR Apache-2.0 |
| `flate2` | `1.1.9` (exact; pure Rust backend) | Inflate local Kraken and generic-ledger XLSX ZIP members without extracting files or starting a subprocess; the standard library has no DEFLATE decoder | MIT OR Apache-2.0 |

| `bitcoin_hashes` | `0.14.101` | HASH160/SHA-256 for replaying legacy public HTLC witness evidence; std and existing SHA-256 code do not provide RIPEMD160. Pure hashing only; no transport or key handling | CC0-1.0 |
| `maturin` | `1.15.0` (exact, build only) | Builds the tax engine's Python binding during `uv sync --locked`; not shipped | MIT OR Apache-2.0 |

## Notable downstream license note

The September 2026 security refresh uses frontend overrides to select patched
compatible versions for existing router, provider-broker and build-tool
dependencies, rather than adding new frameworks. Device sync was removed on
2026-09-30 together with its `spake2`, `zeroconf` and `qrcode` dependencies;
`cryptography` remains only as a test dependency.
The package-manager lockfiles remain the authoritative transitive inventory.

### Optional document tools (not bundled)

Document OCR import can invoke an already installed `pdfinfo`/`pdftoppm` from
[Poppler](https://poppler.freedesktop.org/) to render explicitly selected local
PDF pages. Poppler is GPL-licensed; consult the actual distributor's COPYING
files and upstream license terms before any future bundling. No native binary
is copied into Kassiber, and the import fails explicitly when the tools are
missing.

RP2's `pycountry` dependency (LGPL-2.1-only) no longer ships: packaged builds
stopped bundling RP2 when Kassiber's own tax engine replaced it. It remains
only in development environments, through the RP2 test oracle.

## Bundled source data

Kassiber also bundles a small Bitcoin-only subset of Kraken offline history for
daily OHLCVT values used as local fallback pricing data.

| Data | Files | Role | License / redistribution status |
| --- | --- | --- | --- |
| Kraken BTC daily OHLCVT history | `kassiber/data/rates/kraken/btc_daily/*.csv` | Offline fallback rates for `BTC-EUR` and `BTC-USD` | Unknown / not specified in the local export; review before public release redistribution |

## Development and test infrastructure

The public BIP174/370 PSBT vectors (Ava Chow) and BIP78 Payjoin vectors
(Nicolas Dorier) under `tests/fixtures/chain_analysis/` retain their BSD-2-Clause
license and attribution in that directory's README. The partition solver is
implemented independently; no am-i-exposed/Dojo solver code or entity dataset
is vendored. Research/reference vectors do not supply runtime attribution data.

The regtest Docker harness can build a local Sparrow Frigate image for Silent
Payments protocol testing. It is not a Kassiber runtime dependency.

| Project | Files | Role | License |
| --- | --- | --- | --- |
| [Sparrow Frigate](https://github.com/sparrowwallet/frigate) | `dev/regtest/Dockerfile.frigate`, `dev/regtest/compose.bitcoin.yml` | Optional regtest Electrum server for BIP352 Silent Payments discovery | Apache-2.0 |
| [pytest-xdist](https://github.com/pytest-dev/pytest-xdist) | `pyproject.toml`, `uv.lock`, `.github/workflows/ci.yml` | Runs the explicitly safe Python CI shards across isolated worker processes; socket/process-sensitive modules stay serial | MIT |
| [esbuild](https://github.com/evanw/esbuild) | `ui-tauri/package.json`, `ui-tauri/provider-broker/` | Bundles the TypeScript chat provider broker into one Python-package data file; it is a build dependency, not a provider runtime | MIT |
| [happy-dom](https://github.com/capricorn86/happy-dom), [Testing Library](https://github.com/testing-library/react-testing-library) | `ui-tauri/package.json`, `*.dom.test.tsx` | Mounted desktop-UI tests: a DOM for React effects to run in, and `renderHook`/`act` to drive them. Opt-in per file; the default suite stays static-render on Node | MIT |

## Adapted source code

Some code is adapted from other open-source projects rather than installed as
a dependency. Each adapted file names its origin in a header comment.

| Project | Files | Role | License |
| --- | --- | --- | --- |
| [T3 Code](https://github.com/pingdotgg/t3code) | `ui-tauri/src/components/ai/modelPickerSearch.ts`, `modelPickerKeys.ts`, `composerPromptHistory.ts`, `ModelPickerContent.tsx`, `ModelPickerSidebar.tsx`, `ModelListRow.tsx`, `ComposerControl.tsx`, `providerIdentity.ts` (provider initials), the send/stop marks and action-button styles in `ui-tauri/src/components/ai-02.tsx`, `ui-tauri/src/components/kb/settings/AiProviderCard.tsx` and `aiProviderStatus.ts`, and the setup wizard's `ui-tauri/src/components/kb/Onboarding/stepper.tsx`, `frame.tsx` and the option card in `fields.tsx` | Provider/model picker layout, ranked model search, keyboard navigation, favorites, composer controls, composer action buttons, ArrowUp prompt recall, the Settings → AI providers cards (card layout and the provider status vocabulary, after T3 Code's provider instance cards and `providerStatus.ts`), and the setup wizard's layout (after T3 Code's welcome wizard, wizard step track and settings rows). Copyright (c) 2026 T3 Tools Inc. | MIT |
| [Simple Icons](https://simpleicons.org) | The Ollama mark in `ui-tauri/src/components/ai/providerBrandIcons.tsx` | Brand mark for Ollama providers in the model picker; the mark itself remains Ollama's trademark | CC0-1.0 |
| [RP2](https://github.com/eprbell/rp2), through the [Kassiber-maintained fork](https://github.com/bitcoinaustria/rp2) | `tax-engine/core/src/austria.rs`, `swaps.rs`, and `pool.rs` | Rust translations of the fork's Austrian and moving-average plugins (`plugin/country/at.py`, `plugin/country/at_native_tax_engine.py`, `plugin/accounting_method/moving_average.py`, and `moving_average_at.py`, at the pinned `rp2` commit above): Austrian markers and regimes, swap pairing and the multi-asset runner, and the pool methods. Copyright 2026 bitcoinaustria | Apache-2.0 |
| [oMLX](https://github.com/jundot/omlx) | The oMLX mark in `ui-tauri/src/components/ai/providerBrandIcons.tsx` | Brand mark for oMLX providers in the model picker, from the project's own icon | Apache-2.0 |

## Practical notes

- Third-party provider names and logos shown in the UI are not covered by
  Kassiber's software license. They remain the property of their respective
  owners and are used solely for provider identification; their appearance
  does not imply endorsement, sponsorship, or affiliation.
- Preserve upstream notices and license texts when redistributing Kassiber with bundled third-party code.
- Treat bundled market-data redistribution status as release-blocking until the
  source terms are reviewed.
- If Kassiber adds another direct runtime dependency, add it here with short credit and license info.
- If a dependency introduces a notable licensing constraint, call that out here too.
- For a full release-time dependency inventory, prefer generated tooling or release artifacts over expanding this file by hand.

The Rust chain-client slice reuses the already listed `kassiber-net` transport,
`serde_json` protocol values, `base64` Core authentication, and `sha2` local
credential references. `tempfile` is test-only. It adds no upstream dependency,
license family, networking SDK or runtime installation requirement.

Bitcoin compatibility discovery reuses those internal chain/net clients and
the descriptor dependencies above. The shared clients preserve consent and
proxy policy that raw standard-library sockets would bypass. Only workspace
path dependencies change; no upstream package or license is added.

Bitcoin record composition also reuses the workspace's `kassiber-pydecimal`
for Python-compatible legacy BTC-valued graph fields and half-even satoshi
rounding. The standard library has no exact decimal arithmetic. This adds a
direct internal dependency only, with no new upstream package or license.

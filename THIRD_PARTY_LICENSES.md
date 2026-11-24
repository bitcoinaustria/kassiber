# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber is a Rust workspace (`crates/`, `tax-engine/core`), a native
desktop app (`crates/kassiber-desktop`, its own workspace; see
[Desktop app](#desktop-app-crateskassiber-desktop)) and the earlier Tauri
desktop (`ui-tauri/`). It depends directly on [BDK](https://github.com/bitcoindevkit/bdk)
for watch-only Bitcoin observation, [Liquid Wallet Kit](https://github.com/Blockstream/lwk)
for watch-only Liquid observation, [rust_xlsxwriter](https://github.com/jmcnamara/rust_xlsxwriter)
for XLSX exports, [Typst](https://github.com/typst/typst) for PDF exports,
SQLCipher through `rusqlite` for at-rest encryption, and the `keyring-core`
store crates for opt-in native credential storage. Python, uv and the
packages they pulled in were removed under plan 22.
The descriptor connection screen uses local-only QR scanner libraries for
webcam-based descriptor and BBQR import.
The desktop shell includes Rust keyring crates for AI-provider-key native
storage. This applies only to AI provider API keys; backend tokens, descriptors,
xpubs, blinding keys, and reveal payloads remain SQLCipher-backed.
The R1 Rust runtime reuses those explicit native stores for bound CLI
remembered unlock, with Python-compatible items in its separate namespace.
Its minimal release notifier delegates the GitHub request to the bundled CLI
sidecar rather than carrying a second HTTP client. The desktop and CLI
coordinate update-check consent revocation with an owner-only advisory lock.
Kassiber's own tax engine is a Rust crate (`tax-engine/core`, MIT);
the rows below list the direct Rust dependencies, and the root `Cargo.lock` is
the transitive inventory.
The private custody fusion probe reuses this existing tax-engine crate through
a development dependency; this adds no external package or runtime dependency.
The native journal pre-build reuses Kassiber's transfer crate for component
normalization, revisions, activation and evidence commitments. This internal
dependency adds no third-party package.
The Kassiber-maintained [RP2 fork](https://github.com/bitcoinaustria/rp2), which the engine replaced,
is no longer a dependency: the engine's tests read fixtures it once produced.
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
| `getrandom` | `0.4.3` (exact) | OS randomness for Rust credential enrollment, invalidation generations and backup preview capabilities, the provider broker's tool-call ids, bridge pipe names and private temporary-directory names, and single-use Bitstamp API request nonces | MIT OR Apache-2.0 |
| `bdk_wallet` | `=3.0.0` | Native in-memory watch-only wallet state and indexing matching bdkpython; standard library and descriptor parsing alone cannot reproduce BDK's update model. No BDK storage or connector features | MIT OR Apache-2.0 |
| `bdk_chain` | `=0.23.3` | Native graph, chain checkpoints and changeset merging matching the Python binding; fetched data comes through Kassiber's consent-bound clients | MIT OR Apache-2.0 |
| `bdk_core` | `=0.6.3` | Exact BDK script requests, transaction updates and confirmation anchors; keeps the native observer's update contract pinned to Python | MIT OR Apache-2.0 |
| `ahash` | `0.8.12` (transitive) | BDK's pinned graph/index hash tables require this hasher; selected by the matching SDK rather than a new application API | MIT OR Apache-2.0 |
| `hashbrown` | `0.14.5` (transitive) | BDK's pinned chain/index implementation requires these hash tables; replacing them would fork the reference SDK | MIT OR Apache-2.0 |
| `base64` | `0.21.7` (transitive) | BDK's pinned core serialization dependency; kept alongside existing 0.22 to preserve the matching SDK without patching it | MIT OR Apache-2.0 |
| `lwk_wollet` | `=0.18.0` (exact, default features off) | Native watch-only Liquid wollet state, unblinding and gap discovery matching the Python `lwk` binding; its Electrum, Esplora, Waterfalls, registry and price clients are compiled out and chain data arrives through `kassiber-net` | MIT OR BSD-2-Clause |
| `lwk_common` | `=0.18.1` (exact, default features off) | LWK's network parameters and store traits; no SQLite or network feature | MIT OR BSD-2-Clause |
| `elements` | `0.26.2` (transitive) | LWK 0.18's second Elements dependency (`elements26`), kept alongside 0.25 to use the matching SDK unpatched | CC0-1.0 |
| `aes-gcm-siv`, `bip39`, `fxhash`, `qr_code`, `bmp-monochrome`, `regex-lite`, `tokio-macros` | locked transitive | LWK 0.18's own store encryption, mnemonic, hashing, QR, parsing and runtime-macro dependencies; Kassiber calls none of them directly, and `tokio` has no `net` feature in this tree | MIT OR Apache-2.0 / CC0-1.0 (`bip39`) / MIT (`bmp-monochrome`, `tokio-macros`) |
| `bitcoin_hashes` | `0.14` (already locked) | Python-compatible UUIDv5 review identities using the existing SHA-1 primitive | CC0-1.0 |
| `bitcoin` | `0.32` | Native watch-only BIP32 derivation, key validation, scripts and addresses; cryptography and Bitcoin encodings have no standard-library equivalent | CC0-1.0 |
| `miniscript` | `12` | Native Bitcoin descriptor grammar, multipath expansion and script derivation; avoids a handwritten script interpreter | CC0-1.0 |
| `elements-miniscript` / `elements` | `0.4` / locked transitive `0.25` | Liquid descriptor grammar, SLIP-77 blinding and confidential address derivation; avoids reimplementing confidential address cryptography | CC0-1.0 |
| `secp256k1` / `secp256k1-zkp` | locked transitive | Bitcoin and Liquid public-key operations used by the descriptor libraries; no signing or broadcast API is exposed by Kassiber | CC0-1.0 |
| `regex` | `1` | Bounded non-backtracking descriptor token and wallet-export syntax normalization; already used transitively by backend redaction | MIT OR Apache-2.0 |
| `rust_xlsxwriter` | `0.102.0` (exact, optional features off) | In-memory native XLSX reports, styles and cached formulas; OOXML rendering is unavailable in the standard library | MIT OR Apache-2.0 |
| `zip` | `9.1.3` (exact, default features off, `deflate`) | Writer's ZIP container and in-memory repair of point heights/numeric XML; no archive extraction or new transport | MIT |
| `zlib-rs` / `zopfli` | locked transitive `0.6.8` / `0.8.4` | Writer's pure Rust DEFLATE backends; no system zlib dependency | Zlib / Apache-2.0 |
| `typed-path` / `nix` | locked transitive `0.13.0` / `0.31.3` | ZIP path types and platform support; renderer uses only RAM buffers | MIT OR Apache-2.0 / MIT |
| `crc32fast` / `simd-adler32` | locked transitive `1.5.3` / `0.3.10` | ZIP/compression checksums; updates required by the cached Zopfli dependency | MIT OR Apache-2.0 / MIT |
| `roxmltree` | `0.21` (already locked through Typst) | Read-only parsing of ISO 20022 camt.053 bank statements with DTDs refused and a node limit; no entity resolver, no file or network access. The standard library has no XML parser | MIT OR Apache-2.0 |
| `typst` / `typst-layout` / `typst-pdf` | `=0.15.1` (exact, default features off) | Layout and PDF output for the shared report design (plan 21). A sealed in-memory world: embedded templates and fonts, no package resolver, system font scan, file access or network; the standard library has no layout or PDF engine | Apache-2.0 |
| Typst's transitive crates (`krilla`, `hayro-*`, `rustybuzz`/`ttf-parser`, `icu_*`, `image`, `tiny-skia`, `wasmi`/`wasmparser` and others; 216 locked packages) | locked transitive | Text shaping, Unicode segmentation, image decoding and PDF writing inside the engine; Kassiber exposes no plugin, image-file or font-file input | mostly MIT OR Apache-2.0; also Apache-2.0, MIT, Unicode-3.0 (`icu_*`, `zerovec`, `yoke`, `litemap`), BSD-2-Clause (`arrayref`, `kamadak-exif`, `mutate_once`), BSD-3-Clause (`tiny-skia`), Zlib (`slotmap`), Apache-2.0 WITH LLVM-exception (`wasmparser`, `ar_archive_writer`), 0BSD OR CC0-1.0 (`roman-numerals-rs`), Unlicense OR MIT (`csv`, `byteorder-lite`) |
| `pako` | `^2.1.0` | Zlib/deflate decoding for Better Bitcoin QR descriptor import | MIT |
| `qr-scanner` | `^1.4.2` | Local webcam QR decoding for descriptor-family connection setup | MIT |
| `three` | `0.186.0` (exact) | Glass ribbon 3D view of a transaction graph, loaded only when that view opens; every mesh, material and light is generated locally, with no models, textures or network requests. `@types/three` supplies typings only and is not bundled | MIT |
| `react-markdown` | `^10.1.0` | Markdown renderer for assistant chat replies (paragraphs, lists, code, links) | MIT |
| `remark-gfm` | `^4.0.0` | GitHub-flavored markdown extensions (tables, strikethrough, task lists) for assistant chat replies | MIT |
| `i18next` | `25.8.18` (exact) | Desktop UI localization runtime (English/German, expandable); see [docs/reference/i18n.md](docs/reference/i18n.md) | MIT |
| `react-i18next` | `16.5.8` (exact) | React bindings (hooks/provider) for i18next translations | MIT |
| `dirs` | `6.0.0` | Resolves an absolute per-user home directory before the Tauri app handle exists | MIT OR Apache-2.0 |
| `keyring-core` | `1.0.0` | Rust trait layer for desktop AI-provider storage and runtime CLI remembered unlock; no standard-library native credential API | MIT OR Apache-2.0 |
| `apple-native-keyring-store` | `1.0.0` | macOS Keychain backend for AI provider API keys and opt-in database passphrase remember-unlock | MIT OR Apache-2.0 |
| `block2` | `0.6.2` | Objective-C block bridge for the macOS Touch ID LocalAuthentication callback | MIT |
| `objc2` / `objc2-foundation` / `objc2-app-kit` | `0.6.4` / `0.3.2` / `0.3.2` | Rust Objective-C bridge used for macOS Touch ID prompts and AppKit's main-window frame autosave | MIT |
| `tauri-plugin-window-state` | `~2.4` (Windows and Linux only) | Remembers the main window's size, position, and maximized state across launches, restoring a position only onto a connected monitor. macOS uses AppKit's frame autosave instead | Apache-2.0 OR MIT |
| `tauri-plugin-notification` | `~2.4` | Opt-in OS notifications with fixed generic text, built in the native shell; the webview gets no notification permission. Held on 2.4 because 2.5 and later require a newer Tauri runtime than this lockfile's. Its desktop backend wraps `notify-rust`; the standard library has no OS notification API | Apache-2.0 OR MIT |
| `notify-rust` / `mac-notification-sys` / `tauri-winrt-notification` / `windows` 0.62 family (`windows-core`, `-collections`, `-future`, `-numerics`, `-result`, `-strings`, `-threading`) / `rand` 0.9 / `rand_chacha` / `rand_core` / `ppv-lite86` / `zerocopy` / `zerocopy-derive` | locked transitive | `tauri-plugin-notification`'s desktop backends (macOS NSUserNotificationCenter, Windows toasts, Linux D-Bus through the already locked `zbus`) and its notification-id generator; no network transport | MIT OR Apache-2.0 (`zerocopy`: BSD-2-Clause OR Apache-2.0 OR MIT) |
| `windows-native-keyring-store` | `1.0.0` | Windows user-scope credential backend for AI provider API keys and Python-compatible CLI remembered unlock | MIT OR Apache-2.0 |
| `zbus-secret-service-keyring-store` | `1.0.0` | Explicit Linux Secret Service backend for AI provider API keys and Python-compatible CLI remembered unlock; runtime use is isolated behind `kassiber-secrets/native-keyring`, keeping D-Bus/Tokio out of core-only builds; avoids kernel-keyutils fallback | MIT OR Apache-2.0 |
| `num-bigint` / `num-integer` / `num-traits` | `0.4` / `0.1` / `0.2` | Arbitrary-precision coefficients for the CPython-compatible decimal type (`crates/pydecimal`) the tax engine and the backend crates share, and Python's unbounded ints in `crates/pyjson` | MIT OR Apache-2.0 |
| `serde` / `serde_json` | `1.0.228` / `1` | JSON request/response documents and storage manifests at the tax and custody Python boundary; wallet CLI config intake reuses the locked arbitrary-precision parser because the standard library has no JSON parser; persisted Python text uses `kassiber-pyjson`; custody enables arbitrary integers and correctly rounded float parsing; its CPython codec preserves raw object order; the existing `raw_value` feature retains unported local snapshot fragments during the typed migration, with no new package or lockfile dependency | MIT OR Apache-2.0 |
| `fancy-regex` | `0.20` | Backtracking regexes (over `regex-automata`/`regex-syntax`) for redaction and partition public-watch-material projections, which need lookbehind and lookahead; local attachment filename sanitization reuses the same frozen CPython Unicode class; the provider broker's JavaScript-compatible error redaction needs its backreference | MIT |
| `age` | `0.12` | In-process streaming passphrase encryption for partition and `.kassiber` backups, compatible with Python's pyrage format | MIT OR Apache-2.0 |
| `tar` | `0.4`, default features off (no extended-attribute support) | Partition and `.kassiber` archive encoding/reading; the latter retains Python's count-only attachment manifest | MIT OR Apache-2.0 |
| `rustls` / `rustls-webpki` | `0.23` / locked transitive | TLS and certificate/hostname verification over `kassiber-net`'s guarded stream; the standard library has no TLS implementation. Automatic networking is never delegated to a client SDK | Apache-2.0 OR ISC OR MIT / ISC |
| `ring` / `untrusted` | locked transitive | rustls's explicitly selected cryptographic provider and bounded cryptographic input parsing; no new system TLS library | Apache-2.0 AND ISC / ISC |
| `rustls-pki-types` | locked transitive | PEM/DER parsing for immutable consent-bound trust snapshots; unavailable in the standard library. Roots come from `SSL_CERT_FILE`, a host bundle file, or `webpki-roots`, never from a platform-store loader | Apache-2.0 OR MIT |
| `webpki-roots` | `1` | Embedded Mozilla CA fallback matching the frozen Python certifi bootstrap's intent, without downloading roots at runtime | CDLA-Permissive-2.0 |
| `httparse` | `1.10` | Mature bounded HTTP/1.1 header parser; a full HTTP client could introduce an ungated connector | MIT OR Apache-2.0 |
| `url` / `idna` / `idna_adapter` | `2.5` / locked transitive / `~1.0`, default features off | URL and redirect-origin parsing without DNS; the adapter's ASCII-only mode enforces reviewed A-label host identities without pulling in ICU4X | MIT OR Apache-2.0 |
| `form_urlencoded` / `percent-encoding` / `utf8_iter` / `smallvec` | locked transitive | URL query/escape parsing and IDNA ASCII validation support | MIT OR Apache-2.0 |
| `base64` | `0.22` | Explicit HTTP proxy Basic authentication, separately from target headers; reuses the desktop dependency | MIT OR Apache-2.0 |

| `rusqlite` / `libsqlite3-sys` | `0.40.1` (exact) / locked transitive | Existing-book Rust runtime; SQLite's backup/transaction APIs and bundled SQLCipher are unavailable in the standard library | MIT / MIT (bindings); public domain (SQLite), BSD-style (SQLCipher community) |
| `openssl-src` / `openssl-sys` | locked transitive | Vendored SQLCipher crypto on Linux/Windows avoids an undeclared system OpenSSL runtime; macOS selects CommonCrypto | Apache-2.0 / MIT |
| `sha2` | `0.10` (custody), `0.11` (storage, attachments, backup staging and the provider broker) | Persisted custody evidence/observation commitments, Python-compatible project/schema/remembered-account digests, managed attachment hashes, desktop restore staging commitments and the provider broker's Codex tool-catalog fingerprint; unified with native Secret Service's RustCrypto generation; no standard-library cryptographic hash | MIT OR Apache-2.0 |
| `ssh-key` / `signature` | `=0.6.7` / `=2.2.0`, default features off, `std` and `ed25519` only | In-process `verify-download` signature check replacing the `ssh-keygen -Y verify` subprocess: the SSHSIG signed-data encoding and Ed25519 verification. Kassiber keeps OpenSSH's own armour and field parsing, which the crate's stricter PEM decoder does not reproduce. No key generation, encryption, RSA, ECDSA or I/O | Apache-2.0 OR MIT |
| `ed25519-dalek` / `ed25519` / `ssh-encoding` / `ssh-cipher` / `pem-rfc7468` / `base64ct` | locked transitive (`2.2.0` / `2.2.3` / `0.2.0` / `0.2.0` / `0.7.0` / `1.8.3`) | ssh-key's Ed25519 verifier (canonical-scalar checking, as OpenSSH 10.6 verifies) and its wire, cipher-name and PEM support; the lockfile also lists ssh-key's optional RSA/ECDSA crates, which are not compiled | BSD-3-Clause / Apache-2.0 OR MIT |
| `zeroize` | `1` | Erases application-owned passphrase, fd input and key-literal buffers on drop, including error paths | MIT OR Apache-2.0 |
| `nix` / `libc` | `0.30` / `0.2` | OS-account-home/uid lookup, no-follow file flags, exclusive atomic storage publication unavailable in the standard library, consuming raw passphrase fds without assuming a valid Rust descriptor, test-only journal-loader process CPU timing, and the provider broker's SIGTERM to a tracked agent, errno names and executable lookup | MIT / MIT OR Apache-2.0 |
| `windows-sys` | `0.61` | Windows native ownership SID checks, stable AppData, and Python-compatible file identity; no standard-library owner/SID API | MIT OR Apache-2.0 |
| `rpassword` | `7` | Restoring no-echo controlling-terminal passphrase prompt; the standard library has no portable terminal echo control | Apache-2.0 OR MIT |
| `toml_edit` | `0.25`, parse only (dev only) | Parse Cargo dependency, feature, patch/replace and source policy in the network chokepoint regression scanner; the standard library has no TOML parser, and line matching missed valid Cargo syntax | MIT OR Apache-2.0 |
| `tempfile` | `3` | Private partition staging and atomic publication of exports and machine preferences; disposable Rust unit-test books, metadata and ownership namespaces | MIT OR Apache-2.0 |
| `flate2` | `1.1.9` (exact; pure Rust backend) | Inflate local Kraken and generic-ledger XLSX ZIP members without extracting files or starting a subprocess; the standard library has no DEFLATE decoder | MIT OR Apache-2.0 |

| `bitcoin_hashes` | `0.14.101` | HASH160/SHA-256 for replaying legacy public HTLC witness evidence; std and existing SHA-256 code do not provide RIPEMD160. Pure hashing only; no transport or key handling | CC0-1.0 |

## Desktop app (`crates/kassiber-desktop`)

The desktop app is a native Rust UI built with [Rune](https://github.com/anomalyco/rune)
(MIT, pinned by git revision). It is its own Cargo workspace with its own
`Cargo.lock`, so none of the crates below enter the backend's dependency tree;
the app talks to the bundled `kassiber` binary over the daemon's JSONL protocol.
Rune is built with `default-features = false` and only `accessibility`, `dbus`,
`metal-backend`, `native-file-dialog` and `svg`, which keeps its Ogg/WebM audio
decoders (`symphonia`, `opuscule`, MPL-2.0) out of the lockfile. The app embeds
DM Sans and JetBrains Mono (SIL OFL 1.1, `crates/kassiber-desktop/assets/fonts/`),
Lucide icon outlines (ISC, `packaging/licenses/LUCIDE-LICENSE`) and Rune's own
licence text (`packaging/licenses/RUNE-LICENSE`).

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rune` / `rune-kit` / `rune-notify` | git `f6cf57d` (exact) | Windowing, layout, text, GPU rendering, accessibility, widgets, file dialogs and OS notifications; Rune also patches its forks of `winit`, `parley_engine`, `wgpu-hal`, `accesskit_macos` and `accesskit_windows` at the same revision | MIT |
| `wgpu` / `bytemuck` | `~30.0.2` (default features off) / `1.25` | The transaction graph's 3D glass view, drawn on the device Rune's compositor hands over | MIT OR Apache-2.0 / Zlib OR Apache-2.0 OR MIT |
| `serde` / `serde_json` | `1` | The daemon's JSONL protocol and the app's preference files | MIT OR Apache-2.0 |
| `keyring-core`, `apple-native-keyring-store`, `windows-native-keyring-store`, `zbus-secret-service-keyring-store` / `zeroize` | `1.0` / `1.8` | The same native credential stores and versions as the Tauri shell, so items it wrote keep working; passphrase buffers are cleared | MIT OR Apache-2.0 |
| `miniz_oxide` | `0.8.9` (already locked) | Raw DEFLATE for BBQR wallet-material scans; the standard library has no inflater | MIT OR Zlib OR Apache-2.0 |
| `objc2` / `objc2-foundation` / `block2` / `dispatch2` / `core-foundation` / `security-framework` | macOS only | Touch ID, Keychain access control and the QR scanner's camera queue | MIT / Zlib OR Apache-2.0 OR MIT / MIT OR Apache-2.0 |

The transitive tree for the shipped targets (macOS arm64, Linux x86-64 and
arm64, Windows x86-64) is 398 packages, all under permissive licences:

| License expression | Packages |
| --- | ---: |
| MIT OR Apache-2.0 | 169 |
| MIT | 90 |
| Apache-2.0 OR MIT | 48 |
| Unicode-3.0 | 22 |
| Zlib OR Apache-2.0 OR MIT | 15 |
| MIT/Apache-2.0 | 9 |
| Unlicense OR MIT | 6 |
| MIT OR Apache-2.0 OR Zlib | 5 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 5 |
| Apache-2.0 | 4 |
| Zlib | 4 |
| BSD-3-Clause | 3 |
| BSL-1.0 | 2 |
| MIT OR Zlib OR Apache-2.0 | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| Apache-2.0/MIT | 2 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| BSD-2-Clause | 1 |
| CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | 1 |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | 1 |
| Apache-2.0 AND MIT | 1 |
| ISC | 1 |
| MIT AND Apache-2.0 | 1 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |

No MPL, GPL, LGPL or other copyleft-only package is compiled into the desktop
app. The lockfile still lists one MPL-2.0 crate, `spellbook` 0.4.2: Rune's
spell checker for its web (wasm32) target, which no desktop build compiles.
Should a future Rune revision use it natively, MPL-2.0 is file-level copyleft:
an unmodified copy may ship in the binary, provided its source (the published
crate) stays available to recipients and its notices are kept; any change to
its files must be published under MPL-2.0. `r-efi` (UEFI targets only) is
likewise listed but never built, and is available under MIT OR Apache-2.0.

<details>
<summary>Every package compiled for the shipped targets (398, from <code>cargo metadata --filter-platform</code>)</summary>

| Package | Version | License | Source |
| --- | --- | --- | --- |
| `accesskit` | 0.25.2 | MIT OR Apache-2.0 | crates.io |
| `accesskit_atspi_common` | 0.21.1 | MIT OR Apache-2.0 | crates.io |
| `accesskit_consumer` | 0.39.2 | MIT OR Apache-2.0 | crates.io |
| `accesskit_macos` | 0.27.2 | MIT OR Apache-2.0 | git (Rune) |
| `accesskit_unix` | 0.24.1 | MIT OR Apache-2.0 | crates.io |
| `accesskit_windows` | 0.35.2 | MIT OR Apache-2.0 | git (Rune) |
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | crates.io |
| `aes` | 0.9.4 | MIT OR Apache-2.0 | crates.io |
| `ahash` | 0.8.12 | MIT OR Apache-2.0 | crates.io |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | crates.io |
| `allocator-api2` | 0.2.21 | MIT OR Apache-2.0 | crates.io |
| `apple-native-keyring-store` | 1.0.2 | MIT OR Apache-2.0 | crates.io |
| `arboard` | 3.7.0 | MIT OR Apache-2.0 | crates.io |
| `arrayref` | 0.3.9 | BSD-2-Clause | crates.io |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 | crates.io |
| `as-raw-xcb-connection` | 1.0.1 | MIT OR Apache-2.0 | crates.io |
| `ash` | 0.38.0+1.3.281 | MIT OR Apache-2.0 | crates.io |
| `async-broadcast` | 0.7.2 | MIT OR Apache-2.0 | crates.io |
| `async-channel` | 2.5.0 | Apache-2.0 OR MIT | crates.io |
| `async-executor` | 1.14.1 | Apache-2.0 OR MIT | crates.io |
| `async-io` | 2.6.0 | Apache-2.0 OR MIT | crates.io |
| `async-lock` | 3.5.0 | Apache-2.0 OR MIT | crates.io |
| `async-process` | 2.5.0 | Apache-2.0 OR MIT | crates.io |
| `async-recursion` | 1.2.0 | MIT OR Apache-2.0 | crates.io |
| `async-signal` | 0.2.14 | Apache-2.0 OR MIT | crates.io |
| `async-task` | 4.7.1 | Apache-2.0 OR MIT | crates.io |
| `async-trait` | 0.1.92 | MIT OR Apache-2.0 | crates.io |
| `atomic-waker` | 1.1.2 | Apache-2.0 OR MIT | crates.io |
| `atspi` | 0.29.0 | Apache-2.0 OR MIT | crates.io |
| `atspi-common` | 0.13.0 | Apache-2.0 OR MIT | crates.io |
| `atspi-proxies` | 0.13.0 | Apache-2.0 OR MIT | crates.io |
| `autocfg` | 1.5.1 | Apache-2.0 OR MIT | crates.io |
| `base64` | 0.22.1 | MIT OR Apache-2.0 | crates.io |
| `bit-set` | 0.10.0 | Apache-2.0 OR MIT | crates.io |
| `bit-vec` | 0.9.1 | Apache-2.0 OR MIT | crates.io |
| `bitflags` | 1.3.2 | MIT/Apache-2.0 | crates.io |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | crates.io |
| `blake3` | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | crates.io |
| `block-buffer` | 0.12.1 | MIT OR Apache-2.0 | crates.io |
| `block-padding` | 0.4.2 | MIT OR Apache-2.0 | crates.io |
| `block2` | 0.5.1 | MIT | crates.io |
| `block2` | 0.6.2 | MIT | crates.io |
| `blocking` | 1.7.0 | Apache-2.0 OR MIT | crates.io |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `bytemuck_derive` | 1.12.1 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `byteorder` | 1.5.0 | Unlicense OR MIT | crates.io |
| `byteorder-lite` | 0.1.0 | Unlicense OR MIT | crates.io |
| `bytes` | 1.12.1 | MIT | crates.io |
| `calloop` | 0.13.0 | MIT | crates.io |
| `calloop-wayland-source` | 0.3.0 | MIT | crates.io |
| `cbc` | 0.2.1 | MIT OR Apache-2.0 | crates.io |
| `cc` | 1.11.2 | MIT OR Apache-2.0 | crates.io |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | crates.io |
| `cfg_aliases` | 0.2.2 | MIT | crates.io |
| `cipher` | 0.5.2 | MIT OR Apache-2.0 | crates.io |
| `clipboard-win` | 5.4.3 | BSL-1.0 | crates.io |
| `cmov` | 0.5.4 | Apache-2.0 OR MIT | crates.io |
| `codespan-reporting` | 0.13.1 | Apache-2.0 | crates.io |
| `color_quant` | 1.1.0 | MIT | crates.io |
| `concurrent-queue` | 2.5.0 | Apache-2.0 OR MIT | crates.io |
| `const-oid` | 0.10.3 | Apache-2.0 OR MIT | crates.io |
| `constant_time_eq` | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 | crates.io |
| `core-foundation` | 0.10.1 | MIT OR Apache-2.0 | crates.io |
| `core-foundation` | 0.9.4 | MIT OR Apache-2.0 | crates.io |
| `core-foundation-sys` | 0.8.7 | MIT OR Apache-2.0 | crates.io |
| `core-graphics` | 0.23.2 | MIT OR Apache-2.0 | crates.io |
| `core-graphics-types` | 0.1.3 | MIT OR Apache-2.0 | crates.io |
| `cpubits` | 0.1.1 | MIT OR Apache-2.0 | crates.io |
| `cpufeatures` | 0.3.1 | MIT OR Apache-2.0 | crates.io |
| `crc32fast` | 1.5.5 | MIT OR Apache-2.0 | crates.io |
| `crossbeam-utils` | 0.8.23 | MIT OR Apache-2.0 | crates.io |
| `crypto-common` | 0.2.2 | MIT OR Apache-2.0 | crates.io |
| `ctutils` | 0.4.8 | Apache-2.0 OR MIT | crates.io |
| `cursor-icon` | 1.2.0 | MIT OR Apache-2.0 OR Zlib | crates.io |
| `data-url` | 0.3.2 | MIT OR Apache-2.0 | crates.io |
| `digest` | 0.11.3 | MIT OR Apache-2.0 | crates.io |
| `dispatch` | 0.2.0 | MIT | crates.io |
| `dispatch2` | 0.3.1 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `displaydoc` | 0.2.7 | MIT OR Apache-2.0 | crates.io |
| `dlib` | 0.5.3 | MIT | crates.io |
| `document-features` | 0.2.12 | MIT OR Apache-2.0 | crates.io |
| `downcast-rs` | 1.2.1 | MIT/Apache-2.0 | crates.io |
| `dpi` | 0.1.2 | Apache-2.0 AND MIT | crates.io |
| `endi` | 1.1.1 | MIT | crates.io |
| `enumflags2` | 0.7.12 | MIT OR Apache-2.0 | crates.io |
| `enumflags2_derive` | 0.7.12 | MIT OR Apache-2.0 | crates.io |
| `equivalent` | 1.0.3 | Apache-2.0 OR MIT | crates.io |
| `errno` | 0.3.15 | MIT OR Apache-2.0 | crates.io |
| `error-code` | 3.4.0 | BSL-1.0 | crates.io |
| `euclid` | 0.22.14 | MIT OR Apache-2.0 | crates.io |
| `event-listener` | 5.4.2 | Apache-2.0 OR MIT | crates.io |
| `event-listener-strategy` | 0.5.4 | Apache-2.0 OR MIT | crates.io |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT | crates.io |
| `fax` | 0.2.7 | MIT | crates.io |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 | crates.io |
| `find-msvc-tools` | 0.1.15 | MIT OR Apache-2.0 | crates.io |
| `fixedbitset` | 0.5.9 | MIT OR Apache-2.0 | crates.io |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 | crates.io |
| `float-cmp` | 0.9.0 | MIT | crates.io |
| `foldhash` | 0.1.5 | Zlib | crates.io |
| `foldhash` | 0.2.0 | Zlib | crates.io |
| `font-types` | 0.12.7 | MIT OR Apache-2.0 | crates.io |
| `fontique` | 0.12.0 | Apache-2.0 OR MIT | crates.io |
| `foreign-types` | 0.5.0 | MIT/Apache-2.0 | crates.io |
| `foreign-types-macros` | 0.2.4 | MIT/Apache-2.0 | crates.io |
| `foreign-types-shared` | 0.3.1 | MIT/Apache-2.0 | crates.io |
| `futures-core` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `futures-io` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `futures-lite` | 2.6.1 | Apache-2.0 OR MIT | crates.io |
| `futures-macro` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `futures-task` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `futures-util` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `gethostname` | 1.1.0 | Apache-2.0 | crates.io |
| `getrandom` | 0.3.4 | MIT OR Apache-2.0 | crates.io |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 | crates.io |
| `gif` | 0.14.2 | MIT OR Apache-2.0 | crates.io |
| `gpu-allocator` | 0.28.0 | MIT OR Apache-2.0 | crates.io |
| `half` | 2.7.1 | MIT OR Apache-2.0 | crates.io |
| `harfrust` | 0.12.0 | MIT | crates.io |
| `hashbrown` | 0.15.5 | MIT OR Apache-2.0 | crates.io |
| `hashbrown` | 0.16.1 | MIT OR Apache-2.0 | crates.io |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 | crates.io |
| `hex` | 0.4.3 | MIT OR Apache-2.0 | crates.io |
| `hkdf` | 0.13.0 | MIT OR Apache-2.0 | crates.io |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 | crates.io |
| `hybrid-array` | 0.4.15 | MIT OR Apache-2.0 | crates.io |
| `icu_collections` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_locale_core` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_locale_fallback` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_locale_fallback_data` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_normalizer` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_normalizer_data` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_properties` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_properties_data` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_provider` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_segmenter` | 2.4.0 | Unicode-3.0 | crates.io |
| `icu_segmenter_data` | 2.4.0 | Unicode-3.0 | crates.io |
| `image` | 0.25.10 | MIT OR Apache-2.0 | crates.io |
| `image-webp` | 0.2.4 | MIT OR Apache-2.0 | crates.io |
| `imagesize` | 0.13.0 | MIT | crates.io |
| `indexmap` | 2.14.2 | Apache-2.0 OR MIT | crates.io |
| `inout` | 0.2.6 | MIT OR Apache-2.0 | crates.io |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 | crates.io |
| `jobserver` | 0.1.35 | MIT OR Apache-2.0 | crates.io |
| `keyring-core` | 1.0.1 | MIT OR Apache-2.0 | crates.io |
| `kurbo` | 0.11.3 | Apache-2.0 OR MIT | crates.io |
| `libc` | 0.2.191 | MIT OR Apache-2.0 | crates.io |
| `libloading` | 0.8.9 | ISC | crates.io |
| `libm` | 0.2.16 | MIT | crates.io |
| `linebender_resource_handle` | 0.1.1 | Apache-2.0 OR MIT | crates.io |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | crates.io |
| `linux-raw-sys` | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | crates.io |
| `litemap` | 0.8.3 | Unicode-3.0 | crates.io |
| `litrs` | 1.0.1 | MIT OR Apache-2.0 | crates.io |
| `lock_api` | 0.4.14 | MIT OR Apache-2.0 | crates.io |
| `log` | 0.4.35 | MIT OR Apache-2.0 | crates.io |
| `memchr` | 2.8.3 | Unlicense OR MIT | crates.io |
| `memmap2` | 0.9.12 | MIT OR Apache-2.0 | crates.io |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 | crates.io |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | crates.io |
| `mio` | 1.2.5 | MIT | crates.io |
| `moxcms` | 0.8.1 | BSD-3-Clause OR Apache-2.0 | crates.io |
| `naga` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `naga-types` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `nom` | 8.0.0 | MIT | crates.io |
| `num` | 0.4.3 | MIT OR Apache-2.0 | crates.io |
| `num-bigint` | 0.4.8 | MIT OR Apache-2.0 | crates.io |
| `num-complex` | 0.4.6 | MIT OR Apache-2.0 | crates.io |
| `num-integer` | 0.1.47 | MIT OR Apache-2.0 | crates.io |
| `num-iter` | 0.1.46 | MIT OR Apache-2.0 | crates.io |
| `num-rational` | 0.4.2 | MIT OR Apache-2.0 | crates.io |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 | crates.io |
| `objc-sys` | 0.3.5 | MIT | crates.io |
| `objc2` | 0.5.3 | MIT | crates.io |
| `objc2` | 0.6.5 | MIT | crates.io |
| `objc2-app-kit` | 0.2.2 | MIT | crates.io |
| `objc2-app-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-av-foundation` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-audio` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-audio-types` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-data` | 0.2.2 | MIT | crates.io |
| `objc2-core-foundation` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-graphics` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-image` | 0.2.2 | MIT | crates.io |
| `objc2-core-media` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-text` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-core-video` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-encode` | 4.1.0 | MIT | crates.io |
| `objc2-foundation` | 0.2.2 | MIT | crates.io |
| `objc2-foundation` | 0.3.2 | MIT | crates.io |
| `objc2-io-surface` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-metal` | 0.2.2 | MIT | crates.io |
| `objc2-metal` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `objc2-quartz-core` | 0.2.2 | MIT | crates.io |
| `objc2-quartz-core` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | crates.io |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 | crates.io |
| `ordered-float` | 5.5.1 | MIT | crates.io |
| `ordered-stream` | 0.2.0 | MIT OR Apache-2.0 | crates.io |
| `os_pipe` | 1.2.3 | MIT | crates.io |
| `parking` | 2.2.1 | Apache-2.0 OR MIT | crates.io |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | crates.io |
| `parking_lot_core` | 0.9.12 | MIT OR Apache-2.0 | crates.io |
| `parlance` | 0.1.1 | Apache-2.0 OR MIT | crates.io |
| `parley` | 0.12.0 | Apache-2.0 OR MIT | crates.io |
| `parley_data` | 0.12.0 | Apache-2.0 OR MIT | crates.io |
| `parley_emoji` | 0.12.0 | MIT | crates.io |
| `parley_engine` | 0.12.0 | Apache-2.0 OR MIT | git (Rune) |
| `percent-encoding` | 2.3.2 | MIT OR Apache-2.0 | crates.io |
| `petgraph` | 0.8.3 | MIT OR Apache-2.0 | crates.io |
| `phf` | 0.14.0 | MIT | crates.io |
| `phf_generator` | 0.14.0 | MIT | crates.io |
| `phf_macros` | 0.14.0 | MIT | crates.io |
| `phf_shared` | 0.14.0 | MIT | crates.io |
| `pico-args` | 0.5.0 | MIT | crates.io |
| `pin-project-lite` | 0.2.17 | Apache-2.0 OR MIT | crates.io |
| `piper` | 0.2.5 | MIT OR Apache-2.0 | crates.io |
| `pkg-config` | 0.3.34 | MIT OR Apache-2.0 | crates.io |
| `png` | 0.17.16 | MIT OR Apache-2.0 | crates.io |
| `png` | 0.18.1 | MIT OR Apache-2.0 | crates.io |
| `polling` | 3.11.0 | Apache-2.0 OR MIT | crates.io |
| `pollster` | 1.0.1 | Apache-2.0/MIT | crates.io |
| `portable-atomic` | 1.15.0 | Apache-2.0 OR MIT | crates.io |
| `potential_utf` | 0.1.6 | Unicode-3.0 | crates.io |
| `presser` | 0.3.1 | MIT OR Apache-2.0 | crates.io |
| `proc-macro-crate` | 3.5.0 | MIT OR Apache-2.0 | crates.io |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | crates.io |
| `profiling` | 1.0.18 | MIT OR Apache-2.0 | crates.io |
| `pxfm` | 0.1.30 | BSD-3-Clause OR Apache-2.0 | crates.io |
| `quick-error` | 2.0.1 | MIT/Apache-2.0 | crates.io |
| `quick-xml` | 0.41.0 | MIT | crates.io |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | crates.io |
| `range-alloc` | 0.1.5 | MIT OR Apache-2.0 | crates.io |
| `raw-window-handle` | 0.6.2 | MIT OR Apache-2.0 OR Zlib | crates.io |
| `raw-window-metal` | 1.1.0 | MIT OR Apache-2.0 | crates.io |
| `read-fonts` | 0.41.0 | MIT OR Apache-2.0 | crates.io |
| `regex` | 1.13.1 | MIT OR Apache-2.0 | crates.io |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 | crates.io |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 | crates.io |
| `renderdoc-sys` | 1.1.0 | MIT OR Apache-2.0 | crates.io |
| `resvg` | 0.45.1 | Apache-2.0 OR MIT | crates.io |
| `rfd` | 0.17.2 | MIT | crates.io |
| `rgb` | 0.8.53 | MIT | crates.io |
| `roxmltree` | 0.20.0 | MIT OR Apache-2.0 | crates.io |
| `rune` | 0.0.1 | MIT | git (Rune) |
| `rune-highlights` | 0.0.1 | MIT AND Apache-2.0 | git (Rune) |
| `rune-kit` | 0.0.1 | MIT | git (Rune) |
| `rune-motion` | 0.1.0 | MIT | git (Rune) |
| `rune-notify` | 0.0.1 | MIT | git (Rune) |
| `rune-staging` | 0.0.1 | MIT | git (Rune) |
| `rune-taffy` | 0.0.1 | MIT | git (Rune) |
| `rustc-demangle` | 0.1.28 | MIT/Apache-2.0 | crates.io |
| `rustc-hash` | 1.1.0 | Apache-2.0/MIT | crates.io |
| `rustc-hash` | 2.1.3 | Apache-2.0 OR MIT | crates.io |
| `rustix` | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | crates.io |
| `rustix` | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | crates.io |
| `scoped-tls` | 1.0.1 | MIT/Apache-2.0 | crates.io |
| `scopeguard` | 1.2.0 | MIT OR Apache-2.0 | crates.io |
| `sctk-adwaita` | 0.10.1 | MIT | crates.io |
| `secret-service` | 5.2.0 | MIT OR Apache-2.0 | crates.io |
| `security-framework` | 3.7.0 | MIT OR Apache-2.0 | crates.io |
| `security-framework-sys` | 2.17.0 | MIT OR Apache-2.0 | crates.io |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | crates.io |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | crates.io |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | crates.io |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | crates.io |
| `serde_repr` | 0.1.21 | MIT OR Apache-2.0 | crates.io |
| `sha2` | 0.11.1 | MIT OR Apache-2.0 | crates.io |
| `shlex` | 2.0.1 | MIT OR Apache-2.0 | crates.io |
| `signal-hook-registry` | 1.4.8 | MIT OR Apache-2.0 | crates.io |
| `simd-adler32` | 0.3.10 | MIT | crates.io |
| `simplecss` | 0.2.2 | Apache-2.0 OR MIT | crates.io |
| `siphasher` | 1.0.4 | MIT OR Apache-2.0 | crates.io |
| `skrifa` | 0.44.0 | MIT OR Apache-2.0 | crates.io |
| `slab` | 0.4.12 | MIT | crates.io |
| `slotmap` | 1.1.1 | Zlib | crates.io |
| `smallvec` | 1.16.5 | MIT OR Apache-2.0 | crates.io |
| `smithay-client-toolkit` | 0.19.2 | MIT | crates.io |
| `smol_str` | 0.2.2 | MIT OR Apache-2.0 | crates.io |
| `socket2` | 0.6.5 | MIT OR Apache-2.0 | crates.io |
| `spirv` | 0.4.0+sdk-1.4.341.0 | Apache-2.0 | crates.io |
| `stable_deref_trait` | 1.2.1 | MIT OR Apache-2.0 | crates.io |
| `static_assertions` | 1.1.0 | MIT OR Apache-2.0 | crates.io |
| `strict-num` | 0.1.1 | MIT | crates.io |
| `svgtypes` | 0.15.3 | Apache-2.0 OR MIT | crates.io |
| `swash` | 0.2.10 | Apache-2.0 OR MIT | crates.io |
| `syn` | 2.0.119 | MIT OR Apache-2.0 | crates.io |
| `syn` | 3.0.10 | MIT OR Apache-2.0 | crates.io |
| `synstructure` | 0.14.0 | MIT | crates.io |
| `taffy` | 0.15.1 | MIT | crates.io |
| `termcolor` | 1.4.1 | Unlicense OR MIT | crates.io |
| `thiserror` | 1.0.69 | MIT OR Apache-2.0 | crates.io |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | crates.io |
| `thiserror-impl` | 1.0.69 | MIT OR Apache-2.0 | crates.io |
| `thiserror-impl` | 2.0.21 | MIT OR Apache-2.0 | crates.io |
| `tiff` | 0.11.3 | MIT | crates.io |
| `tiny-skia` | 0.11.4 | BSD-3-Clause | crates.io |
| `tiny-skia-path` | 0.11.4 | BSD-3-Clause | git (Rune) |
| `tiny-skia-path` | 0.11.4 | BSD-3-Clause | crates.io |
| `tinystr` | 0.8.5 | Unicode-3.0 | crates.io |
| `tokio` | 1.54.1 | MIT | crates.io |
| `tokio-macros` | 2.7.2 | MIT | crates.io |
| `toml_datetime` | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 | crates.io |
| `toml_edit` | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 | crates.io |
| `toml_parser` | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | crates.io |
| `tracing` | 0.1.44 | MIT | crates.io |
| `tracing-attributes` | 0.1.31 | MIT | crates.io |
| `tracing-core` | 0.1.36 | MIT | crates.io |
| `tree_magic_mini` | 3.2.2 | MIT | crates.io |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 | crates.io |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | crates.io |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 | crates.io |
| `unicode-width` | 0.2.3 | MIT OR Apache-2.0 | crates.io |
| `usvg` | 0.45.1 | Apache-2.0 OR MIT | crates.io |
| `utf8_iter` | 1.1.0 | Apache-2.0 OR MIT | crates.io |
| `utf_types` | 1.0.0 | Apache-2.0 OR MIT | crates.io |
| `uuid` | 1.29.2 | Apache-2.0 OR MIT | crates.io |
| `version_check` | 0.9.5 | MIT/Apache-2.0 | crates.io |
| `wasmparser` | 0.245.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | crates.io |
| `wayland-backend` | 0.3.17 | MIT | crates.io |
| `wayland-client` | 0.31.15 | MIT | crates.io |
| `wayland-csd-frame` | 0.3.0 | MIT | crates.io |
| `wayland-cursor` | 0.31.14 | MIT | crates.io |
| `wayland-protocols` | 0.32.13 | MIT | crates.io |
| `wayland-protocols-plasma` | 0.3.12 | MIT | crates.io |
| `wayland-protocols-wlr` | 0.3.12 | MIT | crates.io |
| `wayland-scanner` | 0.31.11 | MIT | crates.io |
| `wayland-sys` | 0.31.11 | MIT | crates.io |
| `weezl` | 0.1.12 | MIT OR Apache-2.0 | crates.io |
| `wgpu` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `wgpu-core` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `wgpu-core-deps-apple` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `wgpu-core-deps-windows-linux-android` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `wgpu-hal` | 30.0.1 | MIT OR Apache-2.0 | git (Rune) |
| `wgpu-naga-bridge` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `wgpu-types` | 30.0.2 | MIT OR Apache-2.0 | crates.io |
| `winapi-util` | 0.1.11 | Unlicense OR MIT | crates.io |
| `windows` | 0.62.2 | MIT OR Apache-2.0 | crates.io |
| `windows-collections` | 0.3.2 | MIT OR Apache-2.0 | crates.io |
| `windows-core` | 0.62.2 | MIT OR Apache-2.0 | crates.io |
| `windows-future` | 0.3.2 | MIT OR Apache-2.0 | crates.io |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 | crates.io |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 | crates.io |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 | crates.io |
| `windows-native-keyring-store` | 1.1.0 | MIT OR Apache-2.0 | crates.io |
| `windows-numerics` | 0.3.1 | MIT OR Apache-2.0 | crates.io |
| `windows-registry` | 0.6.1 | MIT OR Apache-2.0 | crates.io |
| `windows-result` | 0.4.1 | MIT OR Apache-2.0 | crates.io |
| `windows-strings` | 0.5.1 | MIT OR Apache-2.0 | crates.io |
| `windows-sys` | 0.52.0 | MIT OR Apache-2.0 | crates.io |
| `windows-sys` | 0.59.0 | MIT OR Apache-2.0 | crates.io |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 | crates.io |
| `windows-targets` | 0.52.6 | MIT OR Apache-2.0 | crates.io |
| `windows-threading` | 0.2.1 | MIT OR Apache-2.0 | crates.io |
| `windows_x86_64_msvc` | 0.52.6 | MIT OR Apache-2.0 | crates.io |
| `winit` | 0.30.13 | Apache-2.0 | git (Rune) |
| `winnow` | 1.0.5 | MIT | crates.io |
| `wl-clipboard-rs` | 0.9.5 | MIT OR Apache-2.0 | crates.io |
| `writeable` | 0.6.4 | Unicode-3.0 | crates.io |
| `x11-dl` | 2.21.0 | MIT | crates.io |
| `x11rb` | 0.13.2 | MIT OR Apache-2.0 | crates.io |
| `x11rb-protocol` | 0.13.2 | MIT OR Apache-2.0 | crates.io |
| `xcursor` | 0.3.11 | MIT | crates.io |
| `xkbcommon` | 0.9.0 | MIT | crates.io |
| `xkbcommon-dl` | 0.4.2 | MIT | crates.io |
| `xkeysym` | 0.2.1 | MIT OR Apache-2.0 OR Zlib | crates.io |
| `xmlwriter` | 0.1.0 | MIT | crates.io |
| `yazi` | 0.2.2 | Apache-2.0 OR MIT | crates.io |
| `yeslogic-fontconfig-sys` | 6.0.1 | MIT | crates.io |
| `yoke` | 0.8.4 | Unicode-3.0 | crates.io |
| `yoke-derive` | 0.8.4 | Unicode-3.0 | crates.io |
| `zbus` | 5.19.0 | MIT | crates.io |
| `zbus` | 6.3.0 | MIT | crates.io |
| `zbus-lockstep` | 0.5.2 | MIT | crates.io |
| `zbus-lockstep-macros` | 0.5.2 | MIT | crates.io |
| `zbus-secret-service-keyring-store` | 1.0.1 | MIT OR Apache-2.0 | crates.io |
| `zbus_macros` | 5.19.0 | MIT | crates.io |
| `zbus_macros` | 6.3.0 | MIT | crates.io |
| `zbus_names` | 4.3.5 | MIT | crates.io |
| `zbus_utils` | 6.1.0 | MIT | crates.io |
| `zbus_xml` | 5.2.1 | MIT | crates.io |
| `zcheapstr` | 1.2.0 | MIT | crates.io |
| `zeno` | 0.3.3 | Apache-2.0 OR MIT | crates.io |
| `zerocopy` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | crates.io |
| `zerocopy-derive` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | crates.io |
| `zerofrom` | 0.1.8 | Unicode-3.0 | crates.io |
| `zerofrom-derive` | 0.1.8 | Unicode-3.0 | crates.io |
| `zeroize` | 1.9.1 | Apache-2.0 OR MIT | crates.io |
| `zerotrie` | 0.2.5 | Unicode-3.0 | crates.io |
| `zerovec` | 0.11.9 | Unicode-3.0 | crates.io |
| `zerovec-derive` | 0.11.7 | Unicode-3.0 | crates.io |
| `zlib-rs` | 0.6.8 | Zlib | crates.io |
| `zmij` | 1.0.23 | MIT | crates.io |
| `zruntime` | 0.2.3 | MIT | crates.io |
| `zune-core` | 0.5.3 | MIT OR Apache-2.0 OR Zlib | crates.io |
| `zune-jpeg` | 0.5.15 | MIT OR Apache-2.0 OR Zlib | crates.io |
| `zvariant` | 5.15.1 | MIT | crates.io |
| `zvariant_derive` | 5.15.1 | MIT | crates.io |
| `zvariant_utils` | 4.2.0 | MIT | crates.io |

</details>

`crates/kassiber-desktop/packaging/licenses/inventory.sh` regenerates a per-host
inventory, with flags for git, unknown and copyleft entries, from local data only.

## Notable downstream license note

The September 2026 security refresh uses frontend overrides to select patched
compatible versions for existing router and build-tool dependencies, rather
than adding new frameworks; the provider-broker ones left with the TypeScript
broker ([plan 23](docs/plan/23-rust-provider-broker.md)). Device sync was removed on
2026-09-30 together with its `spake2`, `zeroconf` and `qrcode` dependencies.
The package-manager lockfiles remain the authoritative transitive inventory.

### Optional document tools (not bundled)

Document OCR import can invoke an already installed `pdfinfo`/`pdftoppm` from
[Poppler](https://poppler.freedesktop.org/) to render explicitly selected local
PDF pages. Poppler is GPL-licensed; consult the actual distributor's COPYING
files and upstream license terms before any future bundling. No native binary
is copied into Kassiber, and the import fails explicitly when the tools are
missing.

RP2's `pycountry` dependency (LGPL-2.1-only) no longer ships or installs:
RP2 left with the Python reference (plan 22).

## Bundled source data

Kassiber also bundles a small Bitcoin-only subset of Kraken offline history for
daily OHLCVT values used as local fallback pricing data.

| Data | Files | Role | License / redistribution status |
| --- | --- | --- | --- |
| Kraken BTC daily OHLCVT history | `crates/rates/data/kraken/btc_daily/*.csv` | Offline fallback rates for `BTC-EUR` and `BTC-USD` | Unknown / not specified in the local export; review before public release redistribution |
| Report fonts | `crates/pdf/fonts/*.ttf`, `crates/pdf/fonts/*.otf` | DM Sans 4.004 (googlefonts/dm-fonts) and JetBrains Mono 2.304 (JetBrains release), unmodified, with Liberation Sans as glyph fallback and Noto Sans JP 2.004 Regular (notofonts/noto-cjk release `Sans2.004`, the Japanese subset OTF) for kana and kanji, embedded in Rust PDF reports; source URLs, versions and SHA-256 in `crates/pdf/fonts/README.md` | SIL OFL 1.1 |

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
| [serde_yaml](https://github.com/dtolnay/serde-yaml) | `crates/repo-checks/Cargo.toml` (dev only), `0.9.34`, locked transitive of Typst | Parses the GitHub Actions workflows for the repository's action pin-policy test; not linked into any shipped binary | MIT OR Apache-2.0 |
| [happy-dom](https://github.com/capricorn86/happy-dom), [Testing Library](https://github.com/testing-library/react-testing-library) | `ui-tauri/package.json`, `*.dom.test.tsx` | Mounted desktop-UI tests: a DOM for React effects to run in, and `renderHook`/`act` to drive them. Opt-in per file; the default suite stays static-render on Node | MIT |

## Adapted source code

Some code is adapted from other open-source projects rather than installed as
a dependency. Each adapted file names its origin in a header comment.

| Project | Files | Role | License |
| --- | --- | --- | --- |
| [T3 Code](https://github.com/pingdotgg/t3code) | `ui-tauri/src/components/ai/modelPickerSearch.ts`, `modelPickerKeys.ts`, `composerPromptHistory.ts`, `ModelPickerContent.tsx`, `ModelPickerSidebar.tsx`, `ModelListRow.tsx`, `ComposerControl.tsx`, `PendingApprovalRow.tsx`, `ComposerModelTraits.tsx`, `ProviderModelPicker.tsx` (the trigger's trait marks, focusing the reasoning levels and the docked layout), `providerIdentity.ts` (provider initials), the send/stop marks and action-button styles in `ui-tauri/src/components/ai-02.tsx`, `ui-tauri/src/components/kb/settings/AiProviderCard.tsx` and `aiProviderStatus.ts`, the add-provider wizard in `ui-tauri/src/components/kb/AiProviderForm.tsx`, the setup wizard's `ui-tauri/src/components/kb/Onboarding/stepper.tsx`, `frame.tsx` and the option card in `fields.tsx`, the right panel, its tabs and surface launcher in `ui-tauri/src/components/kb/assistant-panel/RightPanel.tsx` and its surfaces' frame in `surfaces/SurfaceLayout.tsx` and the sheet surface in `ui-tauri/src/components/ui/sheet.tsx`, the Settings modal in `ui-tauri/src/components/kb/settings/SettingsModal.tsx` and its rail in `SettingsNavigation.tsx`, and the composer surface, message surface, scrollbar and light-theme colour tokens in `ui-tauri/src/styles/globals.css` | Provider/model picker layout, ranked model search, keyboard navigation, favorites, composer controls, composer action buttons, ArrowUp prompt recall, the inline tool-approval row, and the model picker's docked reasoning column, speed toggle and permission-mode menu (after T3 Code's pending-approval row, approval actions, runtime-mode select and `ComposerModelEffort.tsx`), the Settings → Assistant provider rows (row layout and the provider status vocabulary, after T3 Code's provider instance cards and `providerStatus.ts`), the setup wizard's layout (after T3 Code's welcome wizard, wizard step track and settings rows), the docked right panel with its tab strip, `+` menu, surface launcher and maximize control, surface toolbars, the diff-style change list and the log tail (after T3 Code's diff and terminal panels), a sheet on narrow windows (after T3 Code's `RightPanelTabs`, `PreviewPanelShell`, `PanelLayoutControls`, `RightPanelSheet` and sheet popup), Settings as a large modal over the page with a grouped category rail (after T3 Code's `AppRouteModal` and `SettingsSidebarNav`), the redesigned composer outline, sent-message surface, inline code and thin scrollbars, and the light theme's zinc palette (after T3 Code's default light theme). Copyright (c) 2026 T3 Tools Inc. | MIT |
| [Simple Icons](https://simpleicons.org) | The Ollama mark in `ui-tauri/src/components/ai/providerBrandIcons.tsx` | Brand mark for Ollama providers in the model picker; the mark itself remains Ollama's trademark | CC0-1.0 |
| [RP2](https://github.com/eprbell/rp2), through the [Kassiber-maintained fork](https://github.com/bitcoinaustria/rp2) | `tax-engine/core/src/austria.rs`, `swaps.rs`, and `pool.rs` | Rust translations of the fork's Austrian and moving-average plugins (`plugin/country/at.py`, `plugin/country/at_native_tax_engine.py`, `plugin/accounting_method/moving_average.py`, and `moving_average_at.py`, at the pinned `rp2` commit above): Austrian markers and regimes, swap pairing and the multi-asset runner, and the pool methods. Copyright 2026 bitcoinaustria | Apache-2.0 |
| [oMLX](https://github.com/jundot/omlx) | The oMLX mark in `ui-tauri/src/components/ai/providerBrandIcons.tsx` | Brand mark for oMLX providers in the model picker, from the project's own icon | Apache-2.0 |

### T3 Code license

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

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

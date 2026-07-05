# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber currently depends directly on the Kassiber-maintained [RP2 fork](https://github.com/bitcoinaustria/rp2) as its tax engine, [embit](https://github.com/diybitcoinhardware/embit) for descriptor derivation and Liquid support, [XlsxWriter](https://xlsxwriter.readthedocs.io/) for styled XLSX report exports and the generic-ledger import template, [openpyxl](https://openpyxl.readthedocs.io/) for reading filled-in `.xlsx` generic-ledger imports, and [ReportLab](https://www.reportlab.com/) for styled PDF report exports.
The in-development Tauri frontend also depends directly on TanStack Table for
interactive local data grids.
The descriptor connection screen uses local-only QR scanner libraries for
webcam-based descriptor and BBQR import.
The desktop shell includes Rust keyring crates for AI-provider-key native
storage. This applies only to AI provider API keys; backend tokens, descriptors,
xpubs, blinding keys, and reveal payloads remain SQLCipher-backed.

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rp2` | `git+https://github.com/bitcoinaustria/rp2.git@24eeeed5e88d79cedfada9062dbb4fb45f55946c` | Tax engine used by journal processing and tax-aware reports | Apache-2.0 |
| `embit` | `>=0.8.0` | Bitcoin/Liquid descriptor parsing, script derivation, Liquid confidential output handling | MIT |
| `XlsxWriter` | `>=3.2,<4` | Styled `.xlsx` workbook export for practitioner-facing reports and the generic-ledger import template | BSD-2-Clause |
| `openpyxl` | `>=3.1,<4` | Reads filled-in `.xlsx` files for the generic-ledger manual importer | MIT |
| `reportlab` | `>=4.4,<5` | Styled PDF rendering for Austrian and source-of-funds report exports | BSD |
| `sqlcipher3` | `>=0.6.2,<1` | Python binding around SQLCipher 4; wheels bundle a SQLCipher community build for at-rest database encryption | Zlib (binding) + BSD-style (SQLCipher community) |
| `pyrage` | `>=1.3,<2` | In-process `age` implementation used by the `tar | age` backup format when no system `age`/`rage` binary is available | Apache-2.0 / MIT |
| `@tanstack/react-table` | `^8.21.3` | Interactive sorting, filtering, selection, and pagination in desktop UI data tables | MIT |
| `pako` | `^2.1.0` | Zlib/deflate decoding for Better Bitcoin QR descriptor import | MIT |
| `qr-scanner` | `^1.4.2` | Local webcam QR decoding for descriptor-family connection setup | MIT |
| `react-markdown` | `^10.1.0` | Markdown renderer for assistant chat replies (paragraphs, lists, code, links) | MIT |
| `remark-gfm` | `^4.0.0` | GitHub-flavored markdown extensions (tables, strikethrough, task lists) for assistant chat replies | MIT |
| `i18next` | `25.8.18` (exact) | Desktop UI localization runtime (English/German, expandable); see [docs/reference/i18n.md](docs/reference/i18n.md) | MIT |
| `react-i18next` | `16.5.8` (exact) | React bindings (hooks/provider) for i18next translations | MIT |
| `keyring-core` | `1.0.0` | Rust trait layer for desktop AI-provider secret storage | MIT OR Apache-2.0 |
| `apple-native-keyring-store` | `1.0.0` | macOS Keychain backend for AI provider API keys and opt-in database passphrase remember-unlock | MIT OR Apache-2.0 |
| `block2` | `0.6.2` | Objective-C block bridge for the macOS Touch ID LocalAuthentication callback | MIT |
| `objc2` / `objc2-foundation` | `0.6.4` / `0.3.2` | Rust Objective-C bridge used for macOS LocalAuthentication Touch ID prompts | MIT |
| `windows-native-keyring-store` | `1.0.0` | Windows user-scope credential backend for AI provider API keys | MIT OR Apache-2.0 |
| `zbus-secret-service-keyring-store` | `1.0.0` | Linux Secret Service backend for AI provider API keys | MIT OR Apache-2.0 |

## Notable downstream license note

In the current tested RP2 install path, one runtime dependency worth calling out explicitly is:

| Package | Why it matters | License |
| --- | --- | --- |
| `pycountry` | More restrictive than the surrounding MIT/BSD/Apache-style deps in the observed RP2 stack | LGPL-2.1-only |

## Bundled source data

Kassiber also bundles a small Bitcoin-only subset of Kraken offline history for
daily OHLCVT values used as local fallback pricing data.

| Data | Files | Role | License / redistribution status |
| --- | --- | --- | --- |
| Kraken BTC daily OHLCVT history | `kassiber/data/rates/kraken/btc_daily/*.csv` | Offline fallback rates for `BTC-EUR` and `BTC-USD` | Unknown / not specified in the local export; review before public release redistribution |

## Development and test infrastructure

The regtest Docker harness can build a local Sparrow Frigate image for Silent
Payments protocol testing. It is not a Kassiber runtime dependency.

| Project | Files | Role | License |
| --- | --- | --- | --- |
| [Sparrow Frigate](https://github.com/sparrowwallet/frigate) | `dev/regtest/Dockerfile.frigate`, `dev/regtest/compose.bitcoin.yml` | Optional regtest Electrum server for BIP352 Silent Payments discovery | Apache-2.0 |

## Practical notes

- Preserve upstream notices and license texts when redistributing Kassiber with bundled third-party code.
- Treat bundled market-data redistribution status as release-blocking until the
  source terms are reviewed.
- If Kassiber adds another direct runtime dependency, add it here with short credit and license info.
- If a dependency introduces a notable licensing constraint, call that out here too.
- For a full release-time dependency inventory, prefer generated tooling or release artifacts over expanding this file by hand.

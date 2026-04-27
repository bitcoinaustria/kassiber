# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber currently depends directly on the Kassiber-maintained [RP2 fork](https://github.com/bitcoinaustria/rp2) as its tax engine, [embit](https://github.com/diybitcoinhardware/embit) for descriptor derivation and Liquid support, and [XlsxWriter](https://xlsxwriter.readthedocs.io/) for styled XLSX report exports.
The in-development Tauri frontend also depends directly on TanStack Table for
interactive local data grids.

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rp2` | `git+https://github.com/bitcoinaustria/rp2.git@23c944962c775667794b19d66e785058d7aaf599` | Tax engine used by journal processing and tax-aware reports | Apache-2.0 |
| `embit` | `>=0.8.0` | Bitcoin/Liquid descriptor parsing, script derivation, Liquid confidential output handling | MIT |
| `XlsxWriter` | `>=3.2,<4` | Styled `.xlsx` workbook export for practitioner-facing reports | BSD-2-Clause |
| `sqlcipher3` | `>=0.6.2,<1` | Python binding around SQLCipher 4; wheels bundle a SQLCipher community build for at-rest database encryption | Zlib (binding) + BSD-style (SQLCipher community) |
| `pyrage` | `>=1.3,<2` | In-process `age` implementation used by the `tar | age` backup format when no system `age`/`rage` binary is available | Apache-2.0 / MIT |
| `@tanstack/react-table` | `^8.21.3` | Interactive sorting, filtering, selection, and pagination in desktop UI data tables | MIT |
| `react-markdown` | `^10.1.0` | Markdown renderer for assistant chat replies (paragraphs, lists, code, links) | MIT |
| `remark-gfm` | `^4.0.0` | GitHub-flavored markdown extensions (tables, strikethrough, task lists) for assistant chat replies | MIT |

## Notable downstream license note

In the current tested RP2 install path, one runtime dependency worth calling out explicitly is:

| Package | Why it matters | License |
| --- | --- | --- |
| `pycountry` | More restrictive than the surrounding MIT/BSD/Apache-style deps in the observed RP2 stack | LGPL-2.1-only |

## Practical notes

- Preserve upstream notices and license texts when redistributing Kassiber with bundled third-party code.
- If Kassiber adds another direct runtime dependency, add it here with short credit and license info.
- If a dependency introduces a notable licensing constraint, call that out here too.
- For a full release-time dependency inventory, prefer generated tooling or release artifacts over expanding this file by hand.

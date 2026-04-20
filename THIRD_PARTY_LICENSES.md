# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber currently depends directly on the Kassiber-maintained [RP2 fork](https://github.com/bitcoinaustria/rp2) as its tax engine, [embit](https://github.com/diybitcoinhardware/embit) for descriptor derivation and Liquid support, and [PySide6](https://doc.qt.io/qtforpython-6/) for the desktop UI shell.

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rp2` | `git+https://github.com/bitcoinaustria/rp2.git@2fc8592cd952f7aa64669a8ffbd1476dfc48abf9` | Tax engine used by journal processing and tax-aware reports | Apache-2.0 |
| `embit` | `>=0.8.0` | Bitcoin/Liquid descriptor parsing, script derivation, Liquid confidential output handling | MIT |
| `PySide6` | `>=6.7,<7` | PySide6 + QML desktop UI shell and future desktop flows | LGPL-3.0-only |

## Notable downstream license note

In the current tested RP2 install path, one runtime dependency worth calling out explicitly is:

| Package | Why it matters | License |
| --- | --- | --- |
| `pycountry` | More restrictive than the surrounding MIT/BSD/Apache-style deps in the observed RP2 stack | LGPL-2.1-only |

## Practical notes

- Preserve upstream notices and license texts when redistributing Kassiber with bundled third-party code.
- PySide6 is used through the LGPL path; keep the Qt libraries dynamically linked when packaging desktop builds.
- If Kassiber adds another direct runtime dependency, add it here with short credit and license info.
- If a dependency introduces a notable licensing constraint, call that out here too.
- For a full release-time dependency inventory, prefer generated tooling or release artifacts over expanding this file by hand.

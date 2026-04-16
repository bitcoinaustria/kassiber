# Third-Party Licenses

This file is intentionally short. It records the core external projects Kassiber depends on directly and any notable license constraints worth calling out during packaging or redistribution.

It is not meant to be a hand-maintained inventory of every transitive package in every environment.

## Core dependency credit

Kassiber currently depends directly on [RP2](https://github.com/eprbell/rp2) as its tax engine.

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rp2` | `>=1.7.2` | Tax engine used by journal processing and tax-aware reports | Apache-2.0 |

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

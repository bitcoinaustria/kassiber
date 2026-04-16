# Third-Party Licenses

This file tracks the current Kassiber runtime dependency stack at an engineering level.
It is meant to keep packaging and internal distribution auditable, not to replace legal review.

## Kassiber direct runtime dependency

Kassiber currently declares one direct runtime dependency in [pyproject.toml](/Users/dev/Github/kassiber/pyproject.toml):

| Package | Version policy | Role | License |
| --- | --- | --- | --- |
| `rp2` | `>=1.7.2` | Tax engine used by journal processing and tax-aware reports | Apache-2.0 |

## RP2 direct runtime dependencies

The entries below are based on the installed metadata from a reference test environment using `rp2==1.7.2`.

| Package | Observed version | License |
| --- | --- | --- |
| `Babel` | `2.18.0` | BSD-3-Clause |
| `jsonschema` | `4.26.0` | MIT |
| `prezzemolo` | `0.0.4` | Apache-2.0 |
| `python-dateutil` | `2.9.0.post0` | Dual BSD / Apache-2.0 |
| `pycountry` | `26.2.16` | LGPL-2.1-only |
| `pyexcel-ezodf` | `0.3.4` | MIT |

## Reference transitive dependencies

The following packages were resolved transitively in the same reference install:

| Package | Observed version | License |
| --- | --- | --- |
| `attrs` | `26.1.0` | MIT |
| `jsonschema-specifications` | `2025.9.1` | MIT |
| `lxml` | `6.0.4` | BSD-3-Clause |
| `referencing` | `0.37.0` | MIT |
| `rpds-py` | `0.30.0` | MIT |
| `six` | `1.17.0` | MIT |

These versions are a snapshot of the tested install, not a promise that every environment resolves the exact same build. If Kassiber adopts a lockfile or packaged release process, this file should be refreshed from that release artifact.

## Practical compliance notes

- Kassiber is AGPL-3.0-only, but it currently ships with runtime dependencies under Apache-2.0, BSD-style, MIT, and LGPL-2.1-only licenses.
- Preserve upstream license texts and notices when redistributing Kassiber together with vendored or bundled third-party packages.
- `pycountry` is currently the most restrictive runtime dependency in the observed stack because it is LGPL-2.1-only. If Kassiber is distributed beyond local source installs, that dependency should stay part of the release compliance review.
- Update this file whenever a new runtime dependency is added or the RP2 stack changes materially.

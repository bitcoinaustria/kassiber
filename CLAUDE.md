See [AGENTS.md](AGENTS.md) for project shape, architecture, working rules, and verification procedures.

The kassiber skill bundle under [skills/kassiber/](skills/kassiber/) has workflow routing, command references, and gotchas for CLI work. Read the relevant reference file before running non-trivial commands.

The desktop UI (`ui-tauri/`) is bilingual — English and Austrian German (informal `du`) via i18next. When you change a user-facing UI string, update the `en` and `de` resource bundles in lockstep and follow the dev workflow + glossary in [docs/reference/i18n.md](docs/reference/i18n.md) and [docs/reference/i18n-glossary.md](docs/reference/i18n-glossary.md). The CLI and Python daemon stay English/machine-deterministic — do not localize their output.

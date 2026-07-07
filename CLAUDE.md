See [AGENTS.md](AGENTS.md) for project shape, architecture, working rules, and verification procedures.

The Kassiber CLI Agent Skill is maintained outside this repo at https://github.com/bitcoinaustria/kassiber-skill. Use it if installed; otherwise rely on AGENTS.md and the docs in this checkout before running non-trivial commands.

The desktop UI (`ui-tauri/`) is bilingual — English and Austrian German (informal `du`) via i18next. When you change a user-facing UI string, update the `en` and `de` resource bundles in lockstep and follow the dev workflow + glossary in [docs/reference/i18n.md](docs/reference/i18n.md) and [docs/reference/i18n-glossary.md](docs/reference/i18n-glossary.md). The CLI and Python daemon stay English/machine-deterministic — do not localize their output.

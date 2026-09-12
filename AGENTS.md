# Agent contributor guide

Use [CONTRIBUTING.md](CONTRIBUTING.md) for setup, verification, and review.
[README.md](README.md) explains the product; [the plan overview](docs/plan/00-overview.md)
records direction and architecture. [TODO.md](TODO.md) is the execution backlog.
Verify behavior against code when these disagree and update the affected docs.

## Shared boundaries

The CLI and desktop delegate to the same Python core. Keep business decisions
there so the CLI, desktop, and in-product AI cannot produce different accounting.
Read [Python guidance](kassiber/AGENTS.md) for backend work and
[desktop guidance](ui-tauri/AGENTS.md) for React, Tauri, or bridge work. Changes to
a shared contract need verification across its callers, including AI projections.

- Keep observed transactions, ownership evidence, reviewed custody, tax inputs,
  and source-of-funds provenance distinct. A suggestion, graph connection, or
  attached document does not grant ownership, carry basis, or establish tax facts.
- BTC quantities use integer msat. Preserve exact amounts and fail closed on
  ambiguous inputs; unresolved custody must not become a guessed taxable event.
- Preserve the Bitcoin subledger and personal workflows. General accounting is
  a separate opt-in ledger under [plan 17](docs/plan/17-general-accounting-and-private-ai-spec.md),
  delivered through CLI/Agent tools and exact review in the existing Assistant.
  Do not rebuild dedicated accounting routes, navigation, Settings switches, or
  forms. Wallet accounts and RP2 journals are not general-ledger records.
- Keep financial-disclosure authorization separate from task-mutation consent.
  Ordinary accounting AI tools receive opaque task state. Selected financial
  context requires the provider protections and explicit disclosure flow in
  [general accounting](docs/reference/general-accounting.md).
- Keep report-readiness barriers intact. Technical security, correctness, and
  reliability failures block delivery; missing pilot data is tracked separately
  in [the acceptance record](docs/reference/general-accounting-acceptance.md).

## Privacy and authority

Read [privacy and security](docs/reference/privacy-and-security.md) before
changing external I/O, storage, credentials, diagnostics, or AI disclosure.

- Launch, route mounts, display reads, onboarding, status, idle timers, and
  background jobs must not initiate DNS, sockets, HTTP, RPC, or provider probes
  unless the user triggered that exact action or enabled its narrowly scoped
  feature. This includes loopback probes. A seeded or saved backend/provider is
  configuration, not consent. New egress needs documentation and a regression
  proving no traffic before consent.
- AI tools must not expose arbitrary shell/filesystem/CLI dispatch, descriptors,
  xpubs, secrets, env files, raw wallet files, or wallet configuration. Preserve
  explicit allowlists, redacted projections, and book/input-version binding.
- Logs stay in bounded, redacted RAM storage unless the user explicitly exports
  them; see [logging](docs/reference/logging.md). Do not introduce disk logging.
- Keep tests and exploratory commands on disposable data roots. Use the
  [testing guide](docs/reference/testing.md) for regtest resources and the
  distinction between test socket guards and the product no-egress switch.
- Preserve the watch-only boundary: no spending keys or payment signing/broadcasting.
  Sensitive design details belong in their reference docs, not public bug output.

## Working agreements

- For non-trivial CLI, tax, schema, report, or multi-document changes, gather
  evidence and state the requirement, risks, and steps before editing.
- Follow the [quality gate and review workflow](CONTRIBUTING.md#verification-and-review).
  Keep small, coherent commits when committing is in scope.
- Dependency changes need a reason existing code or the standard library cannot
  serve the task, the corresponding lockfile, and concise updates to
  [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). Update installation docs if
  runtime requirements change. Use the package-manager pins in the repo.
- Remote scaffolders and regeneration over existing files require approval for
  that specific run. Do not bypass the pnpm minimum release-age policy without
  owner approval. Keep generated changes separate from handwritten behavior
  when practical. Report accidental replacement of unrelated work immediately.
- Command/flag reshaping is allowed during development without deprecation
  aliases. Update behavior docs, affected AI references, and completed or
  reshaped TODO entries in the same change. Do not grow root instructions into
  a command catalog or a list of shipped features.

## Read by task

These references own the detailed contracts; read the relevant ones before
changing their subsystem.

| Task | Reference |
| --- | --- |
| Tax normalization, transfers, RP2 | [Tax and journals](docs/reference/tax.md), [Austrian handoff](docs/austrian-handoff.md) |
| Custody review and allocations | [Components](docs/reference/custody-components.md), [resolution](docs/reference/custody-resolution.md) |
| Wallet observation and network scope | [Chain observers](docs/reference/chain-observers.md), [book networks](docs/reference/book-networks.md) |
| Local graph analysis or acquisition | [Chain analysis](docs/reference/local-chain-analysis.md), [recurring acquisition](docs/reference/recurring-chain-acquisition.md) |
| Source-of-funds review and exports | [Source-of-funds review](docs/reference/source-of-funds-review.md) |
| Imports and commercial evidence | [Imports](docs/reference/imports.md), [external documents](docs/plan/08-external-document-reconciliation.md) |
| Daemon and AI tools | [Daemon](docs/reference/daemon.md), [AI](docs/reference/ai.md), [operator broker](docs/reference/operator-broker.md) |
| Replication or database changes | [Device sync](docs/reference/device-sync.md), [database compatibility](docs/reference/database-compatibility.md) |
| Lightning adapters | [Lightning discard policy](docs/reference/lightning-opsec.md) |
| Release artifacts and signing | [Prerelease binaries](docs/reference/prerelease-binaries.md), [macOS release](docs/reference/macos-release.md), [Linux packaging](docs/reference/linux-packaging.md) |

## Agent tools

The checked-in CLI skill is [skills/kassiber/SKILL.md](skills/kassiber/SKILL.md);
its helper scripts and intake references form one bundle. The
[regtest skill](skills/kassiber-regtest-mode/SKILL.md) covers the integration harness.
Installed external skills may also be available; verify their commands against
this checkout. The in-product AI's allowlisted references live separately in
[kassiber/ai/skill_references/](kassiber/ai/skill_references/).

Keep shared instructions here, subsystem rules in scoped `AGENTS.md` files,
and procedures in references or skills. `CLAUDE.md` adapters import these files.

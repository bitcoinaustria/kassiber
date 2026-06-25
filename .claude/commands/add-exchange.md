---
description: Add support for a new exchange / broker / custodial platform — guided intake interview, then implementation.
---

Onboard a new exchange into Kassiber by following the playbook in
[skills/kassiber/references/add-exchange.md](../../skills/kassiber/references/add-exchange.md).
Read that file first; it is the source of truth for both halves of this flow.

Steps:

1. **Check it isn't already supported.** Compare against the supported list in
   `docs/reference/imports.md`. If it's there, point the user at the existing
   `wallets import-<slug>` flow instead of building anything. If a spec already
   exists at `docs/exchanges/<slug>.md`, resume from it rather than restarting
   the interview.

2. **Offer the no-code path first** (playbook "Before you build"). If the user
   just needs their own data in, reshaping the export into Kassiber's generic
   columns and using `wallets import-csv` / `import-json` works today with no
   code and no PR. Build a dedicated importer only for repeatable, exact-priced,
   shareable support.

3. **Run the intake interview** (Part 1 of the playbook). Ask, in order:
   name + logo, custodial vs non-custodial, Austrian tax treatment, example
   exports covering all row types, documentation, and API availability. Copy
   `docs/exchanges/TEMPLATE.md` to `docs/exchanges/<slug>.md` and record every
   answer there. Keep secrets and personal data out of the spec and out of chat.

4. **Do not implement from a half-filled spec.** If a required answer is
   missing, stop and ask. An incomplete sample is fine — enumerate row types
   from the docs and make the parser fail-safe on unknowns. Confirm the spec
   with the user before writing code.

5. **Implement** (Part 2 of the playbook): touch the fixed file list, mirror the
   closest existing importer, then verify with `./scripts/quality-gate.sh` and a
   real import round-trip. For `ui-tauri/` catalog/i18n changes, also run
   `pnpm typecheck && pnpm test --run && pnpm lint` from `ui-tauri/`.

6. **Suggest contributing it back** (Part 3): offer to open a PR — a full PR for
   an implemented importer, or a spec-only PR/issue when the user can't code so
   the intake isn't lost. Only open a PR if the user asks.

If the user passed an exchange name as an argument, start (or resume) for that
exchange. Otherwise ask which exchange they want to add.

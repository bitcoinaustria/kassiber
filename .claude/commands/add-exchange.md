---
description: Add support for a new exchange / broker / custodial platform — guided intake interview, then implementation.
---

Onboard a new exchange into Kassiber by following the playbook in
[skills/kassiber/references/add-exchange.md](../../skills/kassiber/references/add-exchange.md).
Read that file first; it is the source of truth for both halves of this flow.

Steps:

1. **Check it isn't already supported.** Compare against the supported list in
   `docs/reference/imports.md`. If it's there, point the user at the existing
   `wallets import-<slug>` flow instead of building anything.

2. **Run the intake interview** (Part 1 of the playbook). Ask, in order:
   name + slug, custodial vs non-custodial, Austrian tax treatment, example
   exports covering all row types, documentation, and API availability. Copy
   `docs/exchanges/TEMPLATE.md` to `docs/exchanges/<slug>.md` and record every
   answer there. Keep secrets and personal data out of the spec and out of chat.

3. **Do not implement from a half-filled spec.** If a required answer or a
   sample export covering all row types is missing, stop and ask for it. Confirm
   the completed spec with the user before writing code.

4. **Implement** (Part 2 of the playbook): touch the fixed file list, mirror the
   closest existing importer, then verify with `./scripts/quality-gate.sh` and a
   real import round-trip. For `ui-tauri/` catalog/i18n changes, also run
   `pnpm typecheck && pnpm test --run && pnpm lint` from `ui-tauri/`.

If the user passed an exchange name as an argument, start the interview for that
exchange. Otherwise ask which exchange they want to add.

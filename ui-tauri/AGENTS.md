# Desktop contributors

These rules supplement [the root guidance](../AGENTS.md). Use the pinned pnpm
version and committed lockfile; frontend commands live in
[CONTRIBUTING.md](../CONTRIBUTING.md#verification-and-review).

## Shared runtime

The React UI reaches the Python daemon through Tauri or the Vite bridge.
Keep accounting, review plans, consent effects, and readiness decisions in the
shared core. Read [the daemon contract](../docs/reference/daemon.md#desktop-invoke-contract)
before changing transport or adding a desktop-invoked kind. A working browser
preview alone does not prove the packaged Tauri path accepts the same request.

- Bind review plans and confirmations to the active book and input version.
  Keep amounts as exact decimal strings and revision timestamps lossless.
  Display the server's plan before explicit confirmation; recompute effects
  server-side rather than treating client state as authorization.
- Transfer/custody navigation is unified in `src/routes/SwapMatching.tsx`;
  component editing lives in `src/routes/transfers-custody/`. Preserve the
  developer-mode gate for gaps/components and the collaborative ownership/fee
  limits in [custody resolution](../docs/reference/custody-resolution.md).
- Use configured backend options for connection setup, not command templates.
  BTCPay provenance mapped onto an existing settlement wallet must not create
  a second balance source for that wallet.
- Keep financial secrets and paths inside their native picker or credential
  flow. The renderer receives staging tokens for chat attachments, never raw
  file-path authority. Source-funds handoffs resume the original scoped task.
- Graph handoffs use `ui.chain_analysis.ai_context`; never interpolate raw
  graph identities into an assistant prompt. Privacy Mirror uses the shared
  analysis result and cannot add an independent detector, graph, or score.

## Interaction and localization

- Use i18next for user-facing strings. Update English and Austrian German
  bundles together, follow [the localization workflow](../docs/reference/i18n.md)
  and [glossary](../docs/reference/i18n-glossary.md), and migrate whole surfaces
  so screens do not become half-translated. CLI/daemon output remains English.
- Treat consent-gated tool cards as updates keyed by `call_id`: the same call
  can appear before consent and again when it starts running. Progress status
  is a loading hint, not chain-of-thought content.
- Route and developer-mode guards must apply to native menu/deep-link entry
  points as well as visible navigation. A hidden link is not an access boundary.
- Report freshness and partial sync errors must remain visible as readiness
  state. A display read cannot silently initiate a wallet/backend sync.

## Preview and packaging

Use the real daemon bridge for interactive verification; the
[testing guide](../docs/reference/testing.md) describes disposable books and
regtest reuse. Preserve another session's active preview and demo resources.

Tauri supervises the bundled runtime in packaged builds. Preserve request-ID
routing, cancellation, streaming, and unsolicited event handling when changing
supervisor code. Read [release packaging](../docs/reference/prerelease-binaries.md)
for sidecar layout and installed-app CLI forwarding; do not assume a source
checkout or separate Python installation exists on the user's machine.

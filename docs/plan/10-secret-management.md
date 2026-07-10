# Secret Management Plan

Date: 2026-07-10

This document records the next desktop secret-handling slice. It is current
truth for the two-boundary model and the AI-provider API-key pilot; backend
tokens, descriptors, xpubs, blinding keys, and reveal payloads remain outside
the OS-store pilot. Desktop Touch ID and CLI remembered unlock are convenience
layers over the SQLCipher passphrase, not new accounting-secret storage
boundaries.

## Boundary Model

Kassiber has two intended secret boundaries:

1. SQLCipher is the at-rest perimeter for the local database, accounting state,
   and intentionally inline secrets. This includes backend credentials,
   descriptors, xpubs, blinding keys, and the current AI-provider API-key
   fallback.
2. OS credential stores are a separate user/device-mediated boundary for
   selected external API secrets and optional unlock convenience. The current
   implementation uses that boundary for AI provider API keys, an opt-in macOS
   Touch ID-gated desktop copy of the SQLCipher passphrase, and an explicitly
   enrolled CLI copy on supported platforms. It does not move
   backend credentials, descriptors, xpubs, blinding keys, or reveal payloads
   out of SQLCipher.

The unlocked Python daemon is the runtime trust boundary. Once it has an open
database connection, it can read any DB-resident secret needed to fulfill an
allowed request. Do not describe Kassiber secrets as encrypted while the daemon
is running.

Out of scope for protection claims:

- malware, compromised browser engine, injected JS, or a compromised assistant
  process running as the user
- admin, root, kernel, debugger, memory inspection, swap capture, or a
  compromised OS account
- production signing, notarization, app attestation, or stronger biometric
  security claims beyond local user-presence convenience
- remote custody, telemetry, crash upload, or outbound secret escrow

Local-first constraints remain unchanged: no telemetry, no remote custody, and
no new outbound calls for secret storage. The assistant/webview still has no
raw shell, raw filesystem, arbitrary CLI, or generic daemon-dispatch access.

## Current Inventory

| Secret or sensitive artifact | Entry | Storage | Transport | Reveal | Logs/diagnostics | Backup/restore | Protection level | Gaps |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| SQLCipher DB passphrase | interactive prompt, `--db-passphrase-fd`, optional desktop Touch ID, optional CLI `remember-unlock` | normally not stored; opt-in per-data-root item in macOS Keychain, Windows Credential Manager, or available unlocked Linux Secret Service | fd/prompt into CLI; CLI credential read only when `cli_remembered_unlock: true`; desktop reads the shared macOS item only after LocalAuthentication succeeds | not revealed | diagnostics redacts passphrase-shaped args and details | required to open backed-up encrypted DB; OS-store copy is not portable backup material | SQLCipher at-rest perimeter only; remembered unlock is convenience | CLI reads are not biometric-gated; biometric reveal gates remain open |
| Backup passphrase / age recipient material | backup CLI prompts/options | not stored by Kassiber | local CLI process | not revealed | diagnostics redaction applies to passphrase-shaped keys/text | user-supplied for each backup/import | external `age` or `pyrage` boundary | no recovery if lost |
| Backend tokens/auth headers/cookies/basic-auth | backend create/update, dotenv migration | SQLCipher DB `backends` table; older dotenvs may still be migrated | daemon/CLI explicit backend flows | `backends.reveal_token` after passphrase round-trip, or explicit plaintext acknowledgement on plaintext DBs | safe backend views expose presence flags only; diagnostics aggregate credential presence | `.kassiber` SQLCipher backup includes values | SQLCipher at rest, unlocked daemon at runtime | not migrated to OS stores in this PR |
| Descriptors, xpubs, blinding keys | wallet create/update/import | SQLCipher DB wallet config today | daemon/CLI wallet flows | `wallets.reveal_descriptor` after passphrase round-trip, or explicit plaintext acknowledgement on plaintext DBs | safe wallet views expose state flags only; diagnostics redacts xpub/xprv patterns | `.kassiber` SQLCipher backup includes values | SQLCipher at rest, unlocked daemon at runtime | still in generic wallet config blob |
| AI provider API keys | CLI stdin/fd/legacy argv, Settings form | SQLCipher inline in `ai_providers.api_key` or OS-backed ref in `ai_provider_secret_refs` | CLI stdin/fd; daemon `ai.providers.set_api_key`; Settings uses narrow daemon kind plus desktop-only native bridge | no reveal kind | provider envelopes omit `api_key`; tool/log/diagnostic redaction tests cover secret-shaped values | SQLCipher-inline keys restore with DB; OS-backed refs restore as repair-needed refs only | SQLCipher or user/device OS store at rest; unlocked daemon at runtime | AI-provider keys only |
| Reveal payloads | reveal daemon kinds | derived from unlocked DB at request time | daemon envelope after passphrase recheck | yes, explicit reveal only | clients must not persist; supervisor/bridge redacts error tails | not separately backed up | UX gate, not cryptographic separation | compromised unlocked daemon can read |
| Sensitive attachments | attachment add/import | copied under managed `attachments/` outside SQLCipher | local file copy/reference only | user opens/manages files | diagnostics omits filenames/URLs | backup format includes managed state tree | filesystem permissions and backup encryption | not DB-encrypted |
| Report/export artifacts | report export commands | managed `exports/` outside SQLCipher | local file writes | user opens/shares | diagnostics outputs are public-safe; reports are not | backup format includes exports depending on pack scope | filesystem permissions and optional backup encryption | user must treat reports as sensitive |
| Diagnostics artifacts | `diagnostics collect`, `--diagnostics-out` | `exports/diagnostics/` or caller path | local JSON | user shares | public-safe sanitizer redacts secret-shaped text/details | included only if backed up by user | sanitized public report | sanitizer cannot prove arbitrary prose is non-sensitive |

User chat text is out of scope for "Kassiber-managed secrets", but prompts can
still include sensitive accounting data and can leave the device when a remote
or CLI provider is selected.

## Leak Table

| Channel | Current code path | Current behavior | Remaining risk |
| --- | --- | --- | --- |
| CLI argv for AI keys | `kassiber/cli/main.py:1359`, `kassiber/cli/main.py:1378`, `kassiber/cli/main.py:2639` | `--api-key-stdin` / `--api-key-fd` are preferred; `--api-key` remains a warning-on-use shim. | Legacy argv values can still land in shell history/process listings. |
| AI provider daemon ingress/envelopes | `kassiber/daemon.py`, `kassiber/ai/providers.py` | `ai.providers.set_api_key` is the only public daemon kind that accepts an API key; create/update/test kinds reject `api_key`; provider payloads include `has_api_key` and `secret_ref.{store_id,state}` only. | The unlocked daemon can still resolve a key at use time to call the provider. |
| AI provider DB schema | `kassiber/db.py`, `kassiber/ai/providers.py` | AI-only `ai_provider_secret_refs` records store refs/state, not secret bytes; OS-backed moves clear `ai_providers.api_key` in the same logical operation after native write success. | `sqlcipher_inline` means the value is still in `ai_providers.api_key`. |
| Missing OS-backed refs | `kassiber/backup/pack.py`, `kassiber/backup/cli.py`, `kassiber/ai/providers.py` | Backup import reports non-inline AI refs as `secret_ref_unavailable`; Settings/use-time native resolution persists `missing` or `unavailable`; use-time access raises the same code with repair details. | OS stores are per-user/per-device and are not included in `.kassiber` backups. |
| Reveal descriptor/token envelopes | `kassiber/daemon.py:6496`, `kassiber/daemon.py:6542`, `kassiber/daemon.py:6626` | Reveal requires an encrypted-DB passphrase round-trip or plaintext-DB acknowledgement and then returns the raw payload. | Reveal is a UX gate, not a second cryptographic boundary. |
| Daemon error envelopes / provider errors | `kassiber/daemon.py:668`, `kassiber/ai/client.py:199` | `_error_envelope` redacts secret-shaped strings and sensitive detail keys before Tauri/Vite/UI egress; AI provider HTTP bodies are treated as hostile, size-limited, and redacted before `error.details.body`. | Error messages can still contain non-secret operational metadata. |
| AI tool previews/results | `kassiber/daemon.py:3383`, `kassiber/daemon.py:3580`, `kassiber/daemon.py:3658`, `kassiber/daemon.py:2659`, `kassiber/daemon.py:3280` | Read-only and mutating previews, streamed tool results, tool-message content, and auto-context entries pass through `redact_tool_arguments`, including oversize fallback summaries. | Read-only business data can still be sent to the configured model. |
| Tauri supervisor stderr/details | `ui-tauri/src-tauri/src/supervisor.rs:78`, `ui-tauri/src-tauri/src/supervisor.rs:83`, `ui-tauri/src-tauri/src/supervisor.rs:104`, `ui-tauri/src-tauri/src/supervisor.rs:909` | Structured error details and daemon stderr tails are redacted before becoming Tauri error payloads. | Runtime process memory and live devtools remain in the runtime boundary. |
| Vite bridge logs/errors | `ui-tauri/vite.config.ts:133`, `ui-tauri/vite.config.ts:323`, `ui-tauri/vite.config.ts:432`, `ui-tauri/vite.config.ts:490` | Development bridge stderr tails and bridge error messages are redacted before JSON/NDJSON error output. | Bridge is development-only and must stay loopback-only. |
| Tauri events / NDJSON streams | `ui-tauri/src/daemon/transport.ts:63`, `ui-tauri/vite.config.ts:490`, `ui-tauri/vite.config.ts:594`, `kassiber/daemon.py:3658` | Stream records use daemon envelopes; tool-result records are redacted before emission. | User-visible report/transaction data can still stream when the user asks for it. |
| Browser/devtools and localStorage | `ui-tauri/src/components/kb/AiProviderForm.tsx:193`, `ui-tauri/src/components/kb/AiProviderForm.tsx:212`, `ui-tauri/src/store/ui.ts:217`, `ui-tauri/src/store/ui.ts:343` | Settings sends API keys via `ai.providers.set_api_key`; persisted UI state excludes logs and notification progress. | The password input is still in live webview memory while the form is open. |
| Diagnostics | `kassiber/diagnostics.py:127`, `kassiber/diagnostics.py:129`, `kassiber/diagnostics.py:609`, `kassiber/diagnostics.py:682` | Public diagnostics redacts sensitive keys, xpub/xprv, bearer tokens, `sk-*`, assigned secret text, details, and paths. | Diagnostics is public-safe by design, not a substitute for reviewing arbitrary logs. |
| Managed paths | `kassiber/core/runtime.py:68`, `kassiber/core/runtime.py:76`, `kassiber/core/runtime.py:77`, `kassiber/core/runtime.py:86` | Data/config/exports/attachments are explicit managed roots. | Attachments and exports are outside SQLCipher unless inside an encrypted backup. |

## Remaining Secret Argv Audit

These forms remain compatibility shims and warn on use. New docs, tests, and
assistant-facing examples should prefer stdin/fd or file paths. Removing the
shims is tracked separately in `TODO.md` because older CLI regression tests and
scripts still exercise them.

| Secret argv form | Parser location | Preferred entry | Current test/call-site evidence |
| --- | --- | --- | --- |
| `backends create/update --auth-header` | `kassiber/cli/main.py:348`, `kassiber/cli/main.py:382` | `--auth-header-stdin` / `--auth-header-fd` | `tests/test_review_regressions.py:2553`, `tests/test_review_regressions.py:2626` |
| `backends create/update --token` | `kassiber/cli/main.py:352`, `kassiber/cli/main.py:386` | `--token-stdin` / `--token-fd` | `tests/test_daemon_smoke.py:1188`, `tests/test_daemon_smoke.py:1341`, `tests/test_review_regressions.py:1272`, `tests/test_review_regressions.py:2555`, `tests/test_review_regressions.py:2628`, `tests/test_review_regressions.py:4334`, `tests/test_review_regressions.py:4445`, `tests/test_review_regressions.py:4571`, `tests/test_review_regressions.py:4748`, `tests/test_review_regressions.py:4900`, `tests/test_review_regressions.py:4934`, `tests/test_review_regressions.py:4992`, `tests/test_review_regressions.py:5038` |
| `backends create/update --username` | `kassiber/cli/main.py:363`, `kassiber/cli/main.py:397` | `--username-stdin` / `--username-fd` | `tests/test_review_regressions.py:2559`, `tests/test_review_regressions.py:2622` |
| `backends create/update --password` | `kassiber/cli/main.py:367`, `kassiber/cli/main.py:401` | `--password-stdin` / `--password-fd` | `tests/test_review_regressions.py:2561`, `tests/test_review_regressions.py:2624` |
| `wallets create --descriptor` | `kassiber/cli/main.py:509` | `--descriptor-stdin` / `--descriptor-fd` / `--descriptor-file` | `tests/test_cli_smoke.py:417`, `tests/test_review_regressions.py:2025`, `tests/test_review_regressions.py:2086` |
| `wallets create --change-descriptor` | `kassiber/cli/main.py:515` | `--change-descriptor-stdin` / `--change-descriptor-fd` / `--change-descriptor-file` | `tests/test_cli_smoke.py:418`, `tests/test_review_regressions.py:2027`, `tests/test_review_regressions.py:2088` |
| `ai providers create/update --api-key` | `kassiber/cli/main.py:1359`, `kassiber/cli/main.py:1378` | `--api-key-stdin` / `--api-key-fd` or Settings `ai.providers.set_api_key` | `tests/test_cli_smoke.py:322` covers the preferred stdin path; no current test should add a raw `--api-key <value>` call site. |

## Target Design

For AI provider keys, the desktop shape is:

1. Store provider metadata in SQLite.
2. Store only a ref row in `ai_provider_secret_refs` for OS-backed keys:
   `provider_name`, `store_id`, `service`, `account`, `state`, timestamps.
3. Derive `service = sha256(bundle_id + ":" + data_root)`, and
   `account = provider_name`.
4. Keep `sqlcipher_inline` as the explicit fallback and restore-compatible
   mode.
5. Surface missing refs as `secret_ref_unavailable` at restore time and use
   time, with `details.refs` and a Settings repair path. Backup export writes
   only non-secret AI provider ref metadata to `manifest.secret_refs`; import
   turns those OS-backed refs into an unavailable warning. Settings and
   use-time resolution persist `missing` or `unavailable` after the first
   failed native lookup so the state is durable after unlock.

This PR implements the schema, redacted envelopes, stdin/fd entry, daemon
rotate/re-enter kind, desktop display, Rust native stores, and the narrow
daemon/supervisor bridge. It intentionally does not migrate backend tokens,
descriptors, xpubs, blinding keys, passphrases, or reveal payloads.

## Platform Policy

| Platform | Default for unsigned/ad-hoc preview builds | OS-store policy | UI copy |
| --- | --- | --- | --- |
| macOS | `sqlcipher_inline` | Keychain can be opt-in experimental while app identity is unsigned, ad-hoc, or unknown. Production-signed builds may default to Keychain later. | "Keychain may ask again after rebuilds or app identity changes." |
| Windows | user-scope Credential Manager / DPAPI when available | User-scope Credential Manager / DPAPI only. No machine-scope secrets. | "Stored for this Windows user account only." |
| Linux | `sqlcipher_inline` when Secret Service is missing, locked, headless, or no D-Bus | Use Secret Service only when available and unlocked. No plaintext fallback. | Show a banner when falling back because no reliable desktop secret service is available. |

Desktop remember-unlock remains macOS-specific: first passphrase entry on the
lock screen can enroll Touch ID for the next unlock, and Settings can verify the
passphrase and store it immediately. Desktop retrieval requires a
LocalAuthentication success before reading the per-data-root Keychain item.

CLI `remember-unlock` is explicitly opt-in on macOS, Windows, and Linux. It
verifies the database passphrase, stores the same service/account item used by
the desktop on macOS, and sets `cli_remembered_unlock: true` in the managed
settings JSON. A desktop-only item is ignored while that marker is absent.
Windows uses user-scope Credential Manager; Linux uses Secret Service only when
available and unlocked. There is no plaintext fallback. CLI reads are not
biometric-gated, and passphrase rotation updates the shared item or disables the
CLI marker and warns if the store rejects the update.

## Rust Secret Store Layer

`ui-tauri/src-tauri/src/secret_store.rs` defines a narrow `SecretStore` trait:
`get`, `set`, `delete`, `list`, and `availability`. Production desktop builds
use platform adapters behind that trait; tests use an in-memory mock store.

Availability is one of:

- `Available { identity_strength: unsigned|adhoc|production|unknown_or_unsigned }`
- `LockedNeedsUnlock`
- `Unavailable { reason }`

The legacy `ProbeSecretStore` remains only as a negative test helper. The
supervisor intercepts daemon-owned `supervisor.ai_secret_store.request`
records and never exposes generic keyring operations to the webview or
assistant.

## Dependency Rationale

Research sources:

- `keyring-core` docs: <https://docs.rs/keyring-core/latest/keyring_core/>
- `keyring-core` topology wiki: <https://github.com/open-source-cooperative/keyring-rs/wiki/Keyring-Core>
- Linux store docs: <https://docs.rs/zbus-secret-service-keyring-store/latest/zbus_secret_service_keyring_store/>
- macOS store docs: <https://docs.rs/apple-native-keyring-store/latest/apple_native_keyring_store/>

Chosen crates are pinned in `ui-tauri/src-tauri/Cargo.toml` and `Cargo.lock`:

```text
keyring-core v1.0.0
`-- log v0.4.29

apple-native-keyring-store v1.0.0
|-- keyring-core v1.0.0
`-- security-framework v3.7.0

windows-native-keyring-store v1.0.0
|-- keyring-core v1.0.0
|-- windows-sys v0.61.2
`-- zeroize v1.8.2

zbus-secret-service-keyring-store v1.0.0
|-- keyring-core v1.0.0
|-- secret-service v5.1.0
`-- zbus v5.15.0
```

`cargo audit` on 2026-05-13 exited 0. It reported 19 allowed warnings in the
existing Tauri/GTK/urlpattern stack, including unmaintained GTK3 binding crates
and non-blocking unsound warnings inherited through Tauri. The new keyring
crates did not introduce a blocking vulnerability.

Tauri Stronghold is rejected for this pass because it would add an
application-managed vault/recovery boundary instead of using user/device OS
credential stores. It also does not solve the unsigned/ad-hoc identity problem
for the current macOS preview, and it would broaden scope beyond the AI-only
pilot.

Keep the current `sqlcipher3` pin unless a concrete SQLCipher blocker appears.

## Flows

### AI Key Set/Use

CLI:

```bash
printf '%s\n' "$OPENAI_API_KEY" | kassiber ai providers create openai \
  --base-url https://api.openai.com/v1 \
  --kind remote \
  --acknowledge \
  --api-key-stdin
```

Desktop:

1. Settings creates/updates provider metadata without `api_key`.
2. If the field contains a new key, Settings sends `ai.providers.set_api_key`.
3. Daemon stores the key inline for this PR and returns only redacted metadata.
4. Chat/model calls resolve the key inside the daemon, not in the webview.

### SQLCipher Unlock

1. The daemon starts locked when the DB is SQLCipher-encrypted.
2. The user supplies the passphrase through the unlock UI or CLI fd path.
3. The Python daemon holds the unlocked connection for runtime requests.
4. If the user enabled Touch ID unlock, the desktop shell stores a per-data-root
   passphrase copy in macOS Keychain either from the first manual lock-screen
   unlock or from the Settings verification dialog, and can retrieve it after
   a LocalAuthentication Touch ID prompt on later lock screens. Passphrase
   rotation updates the stored copy.
5. A one-shot CLI invocation resolves an explicit fd/cached passphrase first,
   then the OS-store copy only when `cli_remembered_unlock` is true, then the
   existing TTY prompt. A stale copy writes `remembered_unlock_stale` to stderr
   and falls through instead of changing machine-mode stdout.

### Reveal Token/Descriptor

1. Client asks `backends.reveal_token` or `wallets.reveal_descriptor`.
2. On encrypted DBs, daemon returns `auth_required`.
3. Client resends with `args.auth_response.passphrase_secret`; on plaintext DBs, it sends `args.auth_response.plaintext_reveal_ack = "COPY LOCAL SECRET"` after the user types that phrase.
4. Encrypted DBs verify by opening a throwaway SQLCipher connection.
5. Daemon returns the reveal payload. Clients must not persist it.

### Restore Missing OS-Backed Secrets

1. SQLCipher-inline keys restore with the DB because the value is inside the
   encrypted database.
2. Future OS-backed refs restore only as refs. `.kassiber` backups record
   `manifest.secret_refs.ai_provider_refs` metadata but must not include
   OS-store secret values. Export refuses a non-inline AI ref if the legacy
   inline `api_key` column is still populated for that provider.
3. `backup import` returns a non-fatal `secret_ref_unavailable` warning with
   `details.refs` when the manifest contains OS-backed refs.
4. After unlock, Settings/use-time native resolution persists non-inline refs
   as `missing` or `unavailable` if the OS store cannot provide the key.
5. If an OS-backed ref is missing or unavailable at use time, reads return
   `secret_ref_unavailable` with `details.refs`.
6. Settings prompts for re-entry and calls `ai.providers.set_api_key`.

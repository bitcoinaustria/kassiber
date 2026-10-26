# Native item compatibility

Baseline: commit `8709d430`, Python `keyring==25.7.0` from `uv.lock`.
The desktop already uses **keyring-core 1.0.0**, apple-native-keyring-store
1.0.0 (`keychain`), windows-native-keyring-store 1.0.0, and
zbus-secret-service-keyring-store 1.0.0 (`rt-tokio-crypto-rust`). It does not
depend on the `keyring` facade. The inspected facade `keyring` 4.0.0 is a
store-selection convenience library; `use_native_store(false)` selects
**kernel keyutils on Linux**, which cannot find Python Secret Service items.
Use the explicit stores already used by the desktop.

| Platform | Python 25.7.0 writes and finds | Rust store default | Configuration to reuse Python items |
| --- | --- | --- | --- |
| macOS | `SecItemAdd` generic password, `kSecAttrService=service`, `kSecAttrAccount=username`, UTF-8 secret. No explicit protected-data-keychain selector or access group. `SecItemCopyMatching` searches by service/account. Python's setter deletes the old matching item before adding. `KEYCHAIN_PATH` is deprecated and ignored by these calls. | `apple_native_keyring_store::keychain::Store::new()` uses the User/default login keychain, generic password service/account. `protected::Store` is a separate store and is inappropriate for these items. | `keychain::Store::new()?.build(service, account, None)` and password APIs; `keychain=User` is the explicit equivalent. Same keys, ordinary default-keychain case. Native ACL permission must be granted to the Rust executable; changing language does not transfer an application's ACL. Python's search list can include other keychains; this store only searches the selected domain. A non-default legacy item needs matching Security.framework search-list behavior or an explicit recovery flow. |
| Windows | `CRED_TYPE_GENERIC` (1). The latest item is at **bare service**, with `UserName=account`. Before every set, any previous bare-service record is copied to `oldAccount@service` (even if the account is unchanged), then bare service is overwritten. Get tries bare service, checks `UserName`, then compound target. Blob is UTF-16LE through WinCred's string conversion. Default `CRED_PERSIST_ENTERPRISE` (3), comment `Stored using python-keyring`. Delete checks both names and their usernames. | `Store::new()` targets **`account.service`**, empty prefix/suffix, divider `.`; Enterprise persistence. A source docstring incorrectly describes alternative configuration defaults; `new_internal` / `new_with_configuration` actually use `.`. `set_password/get_password` use UTF-16LE; `set_secret/get_secret` use raw bytes. | Read explicit `target=service`, validate `get_attributes()["username"]`, else explicit `target=account@service` and validate again. Preserve the found target and `persistence` attribute during updates. Use **password APIs**. For new accounts a compound target avoids replacing another account; optional store config `{prefix:"", divider:"@", suffix:""}` only solves compound names, not the bare-service record. A single default entry is insufficient. |
| Linux Secret Service | Default collection unless `preferred_collection` is configured. Search attributes `{service, username}`; new item attributes `{service, username, application:"Python keyring library"}`, label `Password for 'account' on 'service'`, replace enabled. UTF-8 secret. Default attribute scheme is `service/username`; configurable KeePassXC scheme uses `Title/UserName`. | Default collection for new items. Searches **all collections** by `{service, username}`; no `application` attribute; optional `target` adds an attribute and changes the creation collection. Default label `keyring:account@service`. Existing matches update their secret in place, retaining attributes and label. Multiple matches return Ambiguous. | `Store::new()?.build(service, account, Some({label: python_label}))`, **omit `target`**. Extra `application` is ignored for search and retained on update. To give a newly created item Python's full attributes, call `update_attributes({application:"Python keyring library"})` after set; application is not a build modifier. There is no store setting for restricting searches to Python's preferred collection. Use a collection-aware lower-level Secret Service adapter for non-default collections / alternate schemes, or fail closed on ambiguity. Never add `target=default` to find old items: Python never wrote that attribute. |

An explicit Windows `target` also discards the user/service specifiers inside
the 1.0.0 credential implementation. It preserves an existing item's username
when updating, but creating a missing item this way would initialize an empty
username. New compound items need the configured `divider="@"` store (without
an explicit target), or direct WinCred creation with an explicit UserName.
The spike deliberately returns NoEntry when neither target matches its user;
it never reads or overwrites a mismatched item as a creation fallback.

The runnable [`src/main.rs`](src/main.rs) implements the ordinary Python-item
lookup and in-place update, including explicit Windows targets and persistence.
Its Windows branch is source-backed, **not executed on this Linux host**.
The Windows getter refuses any target whose stored username differs. Errors
other than NoEntry propagate; malformed encodings must not trigger guessed
fallbacks. Deletion in a production adapter must revoke both Windows names,
verify absence as today's product does, and preserve policy markers on failure.

## Kassiber's namespaces and the separate AI format

- Remembered CLI unlock: service `Kassiber CLI Database Passphrase`;
  account = lowercase hex SHA-256 of UTF-8
  `kassiber-cli-remembered-unlock-v2:{database_identity}:{enrollment_id}`.
  Enrollment/policy/database binding remains mandatory. Pre-binding path
  accounts and service `Kassiber Database Passphrase` are cleanup/migration
  input, never automatic unlock authority.
- Native AI references: service = lowercase hex SHA-256 of UTF-8
  `at.bitcoinaustria.kassiber:{resolved canonical data root}`; account =
  provider name stripped and lowercased. Store ID is `macos_keychain`,
  `windows_dpapi`, or `linux_secret_service`; `sqlcipher_inline` remains in DB.
- **Current Python does not write native AI items through Python keyring.**
  `kassiber/ai/providers.py` and the daemon use the desktop's narrow native
  secret control channel. `ui-tauri/src-tauri/src/supervisor.rs` writes
  `secret.as_bytes()` through `NativeSecretStore.set`. On Windows these
  existing AI items are **`account.service`, Local persistence, UTF-8 raw
  bytes**. Keep the desktop's `persistence=Local` and `get_secret/set_secret`
  for this namespace; applying the remembered-unlock UTF-16 adapter to AI
  keys would corrupt them. On Linux the desktop adds label
  `Kassiber AI provider {account}`; on macOS it uses classic keychain.
- Desktop and operator biometric services are separate, item-protected
  contracts. Neither the Python adapter nor generic keyring defaults replace
  them. See [macOS design](../macos-identity/DESIGN.md).

## Local evidence and platform conditions

The session has a D-Bus session bus and an unlocked default collection.
`run.py` created one UUID-scoped synthetic item through Python's actual
SecretService backend, Rust read/updated it, Python read the new value,
and the script verified unchanged object path and attributes before deleting
the item. Provider-added `xdg:schema=org.freedesktop.Secret.Generic` was also
preserved. No existing credential was enumerated or read.

`windows_formats.py` runs the **locked Python Windows backend itself** with
an in-memory WinCred stand-in to pin bare/compound target behavior. It does
not test real WinCred persistence or encoding conversion: the stand-in
models the documented UTF-16LE conversion. Real Mac ACL and Windows WinCred
round trips remain owner validation gates. Linux collection ambiguity,
locked collections and optional schemes need separate adapter acceptance
tests before product cutover. Native-store access must stay on explicit
unlock/secret actions; neither availability reads nor migration inventory
authorize reading credential values.

## Sources

Inspected source from the installed locked wheel and checksum-verified Cargo
archives, not third-party summaries:

- [Python Windows backend](https://github.com/jaraco/keyring/blob/v25.7.0/keyring/backends/Windows.py),
  [SecretService backend](https://github.com/jaraco/keyring/blob/v25.7.0/keyring/backends/SecretService.py),
  [scheme mapping](https://github.com/jaraco/keyring/blob/v25.7.0/keyring/backend.py),
  [macOS Security API](https://github.com/jaraco/keyring/blob/v25.7.0/keyring/backends/macOS/api.py).
- [Rust Windows store source](https://docs.rs/crate/windows-native-keyring-store/1.0.0/source/src/store.rs),
  [Windows credential implementation](https://docs.rs/crate/windows-native-keyring-store/1.0.0/source/src/cred.rs),
  [Secret Service credential implementation](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.0/source/src/cred.rs),
  [classic macOS keychain store](https://docs.rs/crate/apple-native-keyring-store/1.0.0/source/src/keychain.rs),
  [v4 facade source](https://docs.rs/crate/keyring/4.0.0/source/src/lib.rs).
- Product: [unlock_store.py](../../kassiber/secrets/unlock_store.py),
  [AI provider references](../../kassiber/ai/providers.py),
  [desktop store and modifiers](../../ui-tauri/src-tauri/src/secret_store.rs),
  [desktop secret channel](../../ui-tauri/src-tauri/src/supervisor.rs).

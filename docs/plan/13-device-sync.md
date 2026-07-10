# Cross-Device / Multi-User Sync (Untrusted-Storage-First)

**Status:** Implemented 2026-07-10 (S1-S5). This remains the security and
architecture guardrail; operator instructions live in
[docs/reference/device-sync.md](../reference/device-sync.md).
**Driving issue:** [bitcoinaustria/kassiber#309](https://github.com/bitcoinaustria/kassiber/issues/309)
(P1, effort XL) — the issue was rewritten 2026-07-09 to match this design
(mailbox-first, multi-user in scope); if issue and doc drift again, this
doc wins. Related: #285 (backup
GUI), #291 (hash-chained activity log), #276 (Tor egress), #300 (estate
dossier), #302 (reproducible reports), #304 (privacy score).
**Rule for agents:** guardrail/orientation doc. If code lands, keep this
document, AGENTS.md, TODO.md, and issue #309 in lockstep.

## Goal

Keep the devices — and, for organizations, the *people* — of one book in
agreement over time, with:

- **no trusted server**: no Kassiber-operated service, no hosted account,
  no party that can read book contents. Storage, where used, is dumb,
  interchangeable, user-owned ciphertext transport;
- **no silent loss of a financial edit** — concurrent high-stakes conflicts
  surface for blocking review instead of being auto-picked;
- **easy desktop install preserved**: no open ports required, no NAT
  traversal, no Tor bootstrap, no server administration. The default
  remote transport must be "pick a shared folder / paste a storage URL";
- **asynchronous by design**: org peers (client edits Sunday, accountant
  opens Monday) are the primary case; simultaneous online is never
  required;
- the SQLCipher at-rest perimeter intact on every device;
- strictly opt-in: a book that never enables sync has no listener, no
  identities, no behavior change.

**Multi-user is in scope.** Organizations (accountant + client, treasurer
teams) are a driving requirement, not a deferred extension: person-level
identity, roles, and membership are part of the design below.

## Why not the obvious shortcuts

- **Syncing the live SQLite/SQLCipher file** (cloud folder, rsync,
  Syncthing on `data/`): corruption risk (mid-transaction copies, WAL
  mispairing, network-FS locking — sqlite.org/howtocorrupt.html) and
  byte-level last-writer-wins, i.e. silent loss of whole edit sets. Ruled
  out. Note the mailbox transport below may *ride on* a cloud folder — but
  it moves sealed append-only bundle files, never the database.
- **`backup export` / `import` as sync**: whole-DB replace semantics.
  Envelope precedent (`tar | age`, safe-tar, SQLCipher-aware) is reused;
  the restore model is not.
- **cr-sqlite (CRR/CRDT extension)**: maintenance mode (v0.16.x, no
  release since 2024), keeps no history (so no conflict review lane and no
  audit trail), forbids the UNIQUE constraints Kassiber relies on, and
  would colonize derived tables. Ruled out; Kassiber-native authored-event
  substrate instead, which also composes with #291's hash chain.
- **Holepunch/Keet-style tunneling (HyperDHT + Hyperswarm)**: evaluated
  2026-07. Distributed rendezvous with third-party default bootstrap nodes
  (`node*.hyperdht.org`), announces device IP + stable key to public DHT
  participants (geolocation/timing/presence metadata — the #304 leak
  class; Pear's own docs recommend VPN/Tor on top), relays through
  stranger peers under randomizing NATs, JS/Bare-only implementation.
  The key-based-identity + pinned-key *pattern* is retained; the network
  is not.
- **Strict both-online P2P as the org transport** (the original #309
  stance): wrong shape for organizations — the primary workflow is
  asynchronous, and a required inbound listener enlarges the attack
  surface of an accounting app. Kept only as the LAN fast path and an
  optional Tor leg; see "Transport decision".

## Architecture summary

Five layers; merge is transport-agnostic:

1. **Identity: person + device + membership** (schema substrate)
2. **Authored book layer serializer → sealed sync bundles** (`tar | age`
   discipline, per-device recipients, author-signed events)
3. **Replay-not-overwrite importer + conflict review lane + role policy**
4. **Transports:** untrusted encrypted mailbox (primary) · LAN direct
   (fast path) · Tor onion-to-onion (optional) · offline courier (same
   format, manual)
5. **Org membership operations:** sealed invitations, roles, revocation

### 1. Identity, membership, clock

- **Person identity:** each member of a book has an Ed25519 signing
  keypair (`member_id`). Every authored event is signed by its author, so
  Activity attribution is cryptographic — an audit feature in itself.
- **Device identity:** each device holds an X25519 keypair (age-compatible
  recipient) bound to its member via a signed device record. Bundles are
  sealed to device keys; events are signed by member keys. Private keys
  live inside the SQLCipher DB (same boundary as backend tokens), never in
  bundles, never in backups' plaintext manifest.
- New tables: `sync_members` (`member_id`, display name, signing pubkey,
  `role`, `added_at`, `revoked_at`, inviter signature chain) and
  `sync_devices` (`device_id`, `member_id`, recipient pubkey, label,
  `paired_at`, `last_seen_at`, `revoked_at`).
- **Roles:** `owner` (membership changes + edits), `editor` (edits),
  `auditor` (read-only). Role enforcement happens **at merge**: events
  authored by an `auditor`, or by a member revoked before the event's
  clock, are rejected with a visible notice — not just hidden in UI.
- **Clock:** hybrid logical clock (wall time + counter + `replica_id`
  tiebreak) plus a per-replica contiguous sequence number — a version
  vector over a small replica set, enabling "give me everything from
  replica R after seq N" and honest concurrency detection. A replica is a
  (member, device) writer.
- `journal_input_version` stays device-local; journals/reports re-derive
  after ingest, never sync.
- Coordinate ordering with #291: hash chain is per-replica over its own
  authored events; cross-replica order is the HLC. Chained + signed events
  make a malicious storage provider or member tampering detectable.

### 2. What syncs (scope split)

**Authored book layer (syncs):** workspaces, profiles (incl. tax policy),
accounts, wallets' config identity (labels, kind, public descriptors/xpubs
— watch-only material only), authored transaction rows and metadata edits,
`transaction_edit_events`/`_fields`, tags + `transaction_tags`, BIP329
labels, `transaction_pairs` / `direct_swap_payouts` / dismissals /
`loan_legs` / `swap_matching_rules`, source-funds graph (+ immutable
snapshots), external documents + commercial links, saved views,
`attachments` rows and managed blobs, sync membership records themselves
(signed).

**Re-derived locally (never syncs):** journals, quarantines, holdings, tax
summaries, reports, `wallet_utxos` + refresh state, rates cache, freshness
state, transaction graph cache, Lightning snapshots, fetched BTCPay
provenance.

**Local by default (never syncs):** backend secrets and `backends` rows,
AI providers/keys/refs, plaintext `config/backends.env`, logs,
diagnostics, AI chat history, UI pointers, migration stamps, private sync
keys. Peers re-enter backend credentials; at most a "backend X (kind) is
configured elsewhere" hint (no URL/token).

**Never on the wire:** spend keys, seeds, private descriptors, raw wallet
files, DB passphrase, reveal payloads, raw fingerprints (HMAC'd ids only).

### 3. Merge model

- **Event-logged fields** (transaction metadata: note, tags, excluded, tax
  overrides, pricing provenance): union events by UUID, replay in HLC
  order. True concurrency (version-vector, not wall-clock) resolves by
  class:
  - *Add-win, auto-merge:* tags, BIP329 labels, notes — both edits survive
    in Activity; replay order picks the current value.
  - *High-stakes → blocking review:* taxability/exclusion, Austrian
    regime/category, manual pricing, review status, pair/unpair,
    source-funds link review. Both values preserved, a `sync_conflicts`
    row blocks journal processing like a quarantine until a human (with
    edit rights) picks. Never wall-clock LWW.
- **Row-union tables without event logs** (pairs, payouts, dismissals,
  rules, saved views, source-funds graph, external docs): union by UUID;
  deletes become **tombstones** — soft-delete coverage must extend to all
  synced authored tables/joins that today hard-DELETE or cascade.
  Label/name UNIQUE collisions on concurrent creates keep both rows,
  deterministically suffix the newer, and raise a non-blocking notice.
- **Transactions:** chain-derived rows dedupe by `fingerprint`; since the
  fingerprint embeds `wallet_id`, wallet identity replays before
  transaction rows and metadata events (bundle ordering constraint).
  Fingerprint collisions merge onto one UUID (deterministic winner), edit
  events re-anchored by fingerprint. Manually authored rows union by UUID.
- **Idempotence:** ingest is a no-op under duplicate/out-of-order bundles;
  applied ranges recorded in `sync_ingests` (`replica_id`, seq range,
  bundle hash).
- **After ingest:** invalidate journals, recompute locally, refresh report
  blockers. Sync never writes derived tables.

### 4. Bundles and crypto

- Bundle = `tar | age`: `manifest.json` (format version, sender replica,
  version-vector range, prior-bundle hash), `events.jsonl` (signed
  authored events, row snapshots, tombstones; canonical JSON), referenced
  attachment blobs. Reuses `kassiber/backup/age_cli.py` + `safe_tar.py`
  discipline; not the whole-DB pack/restore.
- Sealed to **all current non-revoked device recipients** (age recipient
  mode). Sync keys are independent of the SQLCipher passphrase.
- **Courier-safe by construction** — safe on a found USB stick, therefore
  safe on any dumb storage: this property is what makes the mailbox
  transport (below) nearly free.
- Dedup/idempotency identifiers on the wire are HMACs under a book-scoped
  sync key, never raw `transactions.fingerprint` or attachment `sha256`.
- Attachment blobs travel encrypted, integrity-checked against the row's
  `sha256` after decryption, land atomically with rows/joins.
- A hostile store can read nothing, cannot forge (signatures + #291
  chaining), and gains nothing from replay (idempotent) — it can only
  withhold or delay, which surfaces as visible staleness per peer.

### 5. Transport decision

**Primary: untrusted encrypted mailbox ("bring your own storage").**
Devices exchange sealed bundle files through any dumb blob store the user
or org already has:

- a local folder that happens to be inside Dropbox / Drive / iCloud /
  Nextcloud / Syncthing (Kassiber just reads/writes files; the folder
  service is unwitting ciphertext transport);
- a WebDAV or S3-compatible endpoint (org NAS, Nextcloud, self-hosted or
  rented bucket).

Layout: append-only per-replica bundle files plus a small signed head
pointer per replica; readers poll, verify, ingest. Why this is primary:

- **Asynchronous** — the org workflow works; peers never need to be online
  together.
- **Easy install preserved** — no open ports, no listener, no NAT
  traversal, no Tor bootstrap; "choose a folder / paste a URL" is the
  whole setup.
- **Smaller attack surface** — the app never accepts inbound connections
  for sync.
- **No trusted party** — the store sees ciphertext only; it is
  interchangeable and user-owned; there is no Kassiber service or account.

Honest metadata cost (feed #304): the store operator sees who reads/writes
when, blob sizes/counts, and that Kassiber-shaped files exist — never
contents. Optional bundle padding can blunt size analysis later.

**Fast path: LAN direct P2P.** When devices are co-present, exchange the
same bundles directly: PAKE pairing (QR / short code), pinned device keys,
mDNS with an unlinkable rotating instance name, listener active only while
sync is enabled and the DB is unlocked. Zero infrastructure, instant.

**Optional: Tor onion-to-onion** (after #276) for users who want no third
party touching even ciphertext, accepting both-online-at-once and Tor
availability. Not the default; never required.

**Offline courier:** the same bundle files moved by hand on user-owned
media. Identical format; version vectors keep replay idempotent.

Rejected: any Kassiber-operated relay/rendezvous/account, Nostr transport,
public-DHT announcement transports (Holepunch — see above), iroh's default
public relays (its per-key addressing/PAKE pairing remains good prior
art), and any design that syncs the live DB file.

### 6. Org membership operations

- **Create book sync:** first device becomes `owner`, generates member +
  device keys, initializes the membership chain (self-signed root record).
- **Invite:** owner produces an invitation sealed to the invitee's
  out-of-band public key (QR / file / short-code PAKE handshake over LAN);
  acceptance produces a signed membership record all replicas ingest like
  any authored event.
- **Revoke:** membership record marks the member/device revoked; new
  bundles stop being sealed to their recipients. Honest limitation: a
  revoked member keeps everything already received — revocation is
  forward-secrecy of new edits, not retroactive erasure. High-value
  responses (rotate the book, re-key) are a documented manual procedure.
- **Auditor role:** receives everything, authors nothing that merges.
  Serves the accountant-reviewing-client-books case without giving the
  accountant edit power.

### 7. Desktop / CLI surface

- New `kassiber/core/sync_replication/` module (avoids collision with
  chain-sync `core/sync.py`): event capture, serializer, merger, conflict
  store, membership. Pure logic; transports injected.
- CLI: `kassiber sync {status,enable,disable,transport,invite,join,
  members,devices,push,pull,conflicts {list,resolve}}` — machine envelopes
  with stable `kind`s.
- Desktop: Settings → Sync panel (enable, choose mailbox folder/URL,
  invite/join via QR or code, member/device list with revoke) + a
  conflict-review queue feeding the report-blocker surface. New `ui.sync.*`
  kinds wired through `SUPPORTED_KINDS`, `ALLOWED_DAEMON_KINDS`,
  `ALLOWED_BRIDGE_KINDS` in lockstep; progress via `build_event_envelope`.
  No sync kind is exposed to the AI tool surface.
- i18n: all new desktop strings in `en` + `de` lockstep.

## Phasing

| Phase | Deliverable | Ship gate | Status |
|---|---|---|---|
| S1 | Schema substrate: members/devices/replicas, HLC + per-replica seq on authored events, tombstone coverage, signed events | quality gate + convergence property tests (duplicate/out-of-order replay idempotent) | Shipped |
| S2 | Serializer + importer + conflict lane + role policy, exercised via file-based bundles (courier format) | two temp data-roots converge; auditor edits refused; high-stakes conflicts block reports until resolved | Shipped |
| S3 | Mailbox transport: watched folder + WebDAV/S3 backends, head pointers, staleness surfacing; Settings/Sync UI + invite/join + conflict queue | packaged-app end-to-end through a dumb store; no listener involved | Shipped |
| S4 | LAN direct fast path (PAKE pairing, mDNS, pinned keys) | two machines converge with zero infrastructure | Shipped |
| S5 | Tor onion-to-onion optional leg (after #276); tombstone GC policy | remote sync with no third party; GC never resurrects deletes | Shipped |

S1+S2 are the risk core, fully testable without networking. S3 — not P2P —
is what unlocks the org use case and must not require any org to run or
configure anything beyond storage they already have.

## Implemented decisions and remaining extensions

- **Mailbox layout:** append-only per-replica bundle objects, signed head and
  acknowledgement pointers, opaque book/replica HMAC paths, and owner-attested
  snapshot bundles for new members joining old books.
- **Tombstone GC without membership server:** compaction requires both the
  user-visible horizon (180 days by default, 30-day floor) and signed
  acknowledgement by every non-revoked replica. An offline blocker must
  reconnect, or an owner revokes and re-invites it through a snapshot. Compact
  causal fences remain so stale replay cannot resurrect a row.
- **Passphrase/custody story for orgs:** each device keeps its own SQLCipher
  passphrase; sync never moves it. Book confidentiality across people is
  membership-based, not passphrase-based.
- **Lightning fingerprint drift:** whether a `payment_hash` fallback key
  is needed when independently-synced Lightning rows disagree on
  fingerprint inputs.
- **Conflict UX home:** a dedicated Settings → Device sync queue feeds the
  report-blocker surface; quarantine remains tax-input-specific.
- **Backend hints:** dropped from v1. Backend rows, URLs, tokens, and presence
  hints remain local.
- **Hosted-storage API connectors (e.g. native Google Drive):** v1
  mailbox backends are local folder + WebDAV + S3 only. Drive without its
  desktop sync app is reachable only via Google's proprietary OAuth/REST
  API, which would require the project to register and maintain a verified
  Google OAuth client — an administrative dependency on a vendor, not a
  data-path change (Drive would still hold ciphertext only). Revisit as an
  optional connector behind the same transport interface if org demand
  materializes; until then, Drive works via its desktop client folder or a
  user-side WebDAV bridge/mount.
## Acceptance invariants (restated as tests)

1. Independent replicas converge to identical authored state under
   duplicate, reordered, and interleaved bundle application — regardless
   of transport.
2. Concurrent high-stakes financial edits produce a blocking review item;
   no wall-clock LWW resolves them silently.
3. A delete on one replica is never resurrected by a stale peer.
4. Bundles contain no derived state, no secrets, no raw fingerprints, no
   private descriptors (asserted by schema allowlist, not denylist).
5. Events from auditors or revoked members never merge; attribution of
   every merged event is signature-verified.
6. A hostile mailbox store cannot read book contents, cannot forge or
   reorder history undetected, and withholding surfaces as visible per-peer
   staleness.
7. A user who never enables sync has no listener, no keys, no behavior
   change; a user on mailbox-only sync never opens a listening socket.

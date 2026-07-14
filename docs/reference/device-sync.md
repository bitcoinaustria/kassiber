# Device and Team Sync

Kassiber can replicate one book across devices and people without a Kassiber
account or trusted server. Sync is strictly opt-in and requires an unlocked
SQLCipher database. A book that never enables it has no sync identities, keys,
poller, or listener.

The replicated unit is the signed authored-event layer, not the SQLite file.
Journals, reports, rates, UTXO/freshness caches, BDK/LWK observer state and
coverage, backend rows and credentials,
AI configuration/history, logs, raw wallet files, private descriptors, seeds,
and spend keys stay local. Watch-only public wallet material can sync; raw
transaction fingerprints and attachment hashes are replaced with book-keyed
HMAC identifiers on the wire.

## Recommended setup: encrypted mailbox

The default asynchronous path writes sealed append-only `tar | age` bundles to
storage you control. A local folder may live inside Dropbox, Drive, iCloud,
Nextcloud, or Syncthing; Kassiber sees only a folder. WebDAV and S3-compatible
endpoints are also supported. The store sees access timing, ciphertext sizes,
and object counts, but not book contents or raw identifiers.

Enable sync on the first encrypted device:

```sh
kassiber sync enable --member-name "Alice" --device-label "Laptop"
kassiber sync transport add --kind folder --label Team --path "$HOME/Shared/Kassiber"
kassiber sync push --transport Team
```

For WebDAV, provide the password through stdin or a file descriptor:

```sh
kassiber sync transport add --kind webdav --label NAS \
  --url https://nas.example/remote.php/dav/files/alice/kassiber \
  --username alice --password-fd 3 3<webdav-password.txt
```

S3 uses `--endpoint`, `--bucket`, optional `--region` / `--prefix`,
`--access-key`, and `--secret-key-fd`. Transport credentials live inside the
local SQLCipher boundary and never appear in status output or bundles.

Exchange changes explicitly or from Settings → Device sync:

```sh
kassiber sync pull --transport Team
kassiber sync push --transport Team
kassiber sync status
```

Mailbox-only sync never binds a listening socket. Signed per-replica head
pointers detect rollback/equivocation, signed acknowledgement pointers support
tombstone quorum, and peer status shows last contact and staleness. Storage can
withhold data, but withholding cannot be hidden as freshness.

## Invite a person or device

The joining encrypted device creates its own person/device keys and exports a
signed public request:

```sh
kassiber --machine --output join-request.json sync join-request \
  --member-name "Accountant" --device-label "Office Mac"
```

An owner assigns `owner`, `editor`, or read-only `auditor` and seals an
invitation to that device:

```sh
kassiber sync invite --request join-request.json --role auditor \
  --invitation accountant-invitation.age
```

The joining device accepts it with the request id from `join-request.json`:

```sh
kassiber sync join --request-id REQUEST_ID \
  --invitation accountant-invitation.age
```

The desktop panel performs the same flow with locally generated codes and QR
images. For an established book, the owner then publishes a full checkpoint
sealed to the new device:

```sh
kassiber sync push --transport Team --snapshot
```

Snapshot bundles are owner-attested and carry signed replica chain tips. They
let a late joiner start from current authored state without decrypting old
objects that predate its recipient key. Append-only transaction-edit history is
re-attested into the snapshot.

Revocation stops future bundles from being sealed to that recipient. It cannot
erase ciphertext or book contents the member already received. For a high-risk
departure, rotate into a new book and re-invite the remaining members.

## Merge and conflict rules

Every event has an Ed25519 author signature, per-replica sequence/hash chain,
hybrid logical clock, and version-vector context. Duplicate and out-of-order
delivery is idempotent. Deletes win against stale/concurrent upserts.

Low-risk concurrent metadata keeps both events and converges deterministically.
Concurrent exclusion/taxability, Austrian classification, manual pricing,
review state, pair/unpair, and source-of-funds review changes create a blocking
conflict. Journals and dependent reports stay blocked until an editor chooses a
signed resolution in Settings → Device sync or with:

```sh
kassiber sync conflicts list
kassiber sync conflicts resolve CONFLICT_ID --source-event-id EVENT_ID
```

Auditor-authored edits and events at or after a member's revocation clock are
rejected with visible blocking notices.

## Offline courier

The same sealed format works on user-owned media:

```sh
kassiber sync push --bundle outgoing.kassiber-sync.age
kassiber sync pull --bundle incoming.kassiber-sync.age
```

The file is safe if lost: it is encrypted to active device recipients and all
events remain signed. Never copy or synchronize the live SQLCipher/SQLite file,
its WAL, or the data directory.

## LAN fast path

LAN sync is explicit and single-use. The listening command runs only while the
database is unlocked and sync is enabled, advertises a rotating opaque mDNS
name, performs SPAKE2 with the short-lived offer code, pins both age device
keys, exchanges the same sealed bundles, then closes the listener:

```sh
# Device A
kassiber sync lan listen --offer lan-offer.txt

# Device B
kassiber sync lan connect --offer lan-offer.txt
```

Use `--no-mdns` when the offer file/QR already carries the address, or
`kassiber sync lan discover` to inspect nearby rotating advertisements. The
mailbox remains the primary org workflow; simultaneous online presence is not
required.

## Optional Tor onion leg

Kassiber does not silently install, start, or configure Tor. To avoid even a
ciphertext mailbox, configure a user-controlled Tor v3 onion service that maps
an onion port to a loopback port, then run:

```sh
# Onion-service host; HiddenServicePort must map 443 to 127.0.0.1:18443
kassiber sync tor listen --onion-host YOUR_V3_ADDRESS.onion \
  --onion-port 443 --local-port 18443 --offer tor-offer.txt

# Peer; credentials may be supplied through --tor-proxy-fd
kassiber sync tor connect --offer tor-offer.txt \
  --tor-proxy-fd 3 3<tor-proxy.txt
```

The onion path uses the same SPAKE2 confirmation, pinned device identity, and
sealed bundles. There is no clearnet fallback. It is optional, both-online, and
depends on the user's Tor service rather than a Kassiber relay.

## Tombstone compaction

Compaction is never time-only. A tombstone must be older than the visible
horizon (180 days by default, minimum 30) and acknowledged in signed vectors by
every active non-revoked replica:

```sh
kassiber sync gc status --horizon-days 180
kassiber sync gc run --horizon-days 180 --apply
```

The operation keeps a compact causal delete fence and audit log, so replaying
an old bundle cannot resurrect the row. An offline blocker must reconnect. If
it will not return, an owner revokes that device and re-invites it later through
an owner snapshot; revocation is explicit and never inferred from time alone.

## Custody and recovery

Each device should use its own SQLCipher passphrase. Sync never transfers that
passphrase. Confidentiality between people follows book membership: every
active recipient can decrypt new bundles, regardless of whether their local
database passphrases match. Keep normal encrypted backups on every device;
replication is not a backup replacement.

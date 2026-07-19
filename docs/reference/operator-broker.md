# Operator Unlock Broker

The operator broker is Kassiber's terminal-first way to keep an encrypted
project available for a deliberate work session without giving each CLI
process the database passphrase. It is an authorization convenience over
SQLCipher, not a new encryption layer and not an identity system for agents.

## Principal and threat model

The logged-in operating-system user is the principal. Every process running as
that user can intentionally use an active lease. A lease does not distinguish
Codex, another agent, a shell script, or an interactive terminal, and Kassiber
must never claim otherwise.

The broker preserves these boundaries:

- each OS login user has a separate endpoint, peer identity, native credential
  namespace, broker process, and in-memory lease set;
- each canonical project/database has an independent passphrase, lease,
  capability grant, FIFO queue, worker, and ownership lock;
- the passphrase never appears in argv, environment variables, JSON protocol
  frames, logs, diagnostics, status, or command output;
- SQLCipher still protects the database at rest, and broker death, logout, or
  reboot loses every in-memory brokered lease;
- admin operations require a fresh, single-operation authorization and cannot
  be added to a standing lease;
- in-app AI tool consent remains a separate gate after broker authorization.

Same-user malware, a compromised same-user agent, root/admin, debugger access,
kernel compromise, swap capture, and a compromised OS are outside this
boundary. Python strings, subprocess buffers, SQLCipher bindings, and Rust
strings cannot provide a truthful deterministic-zeroization guarantee. Kassiber
minimizes copies, avoids serialization and logging, closes descriptors and
connections promptly, and drops references on lock or process exit; it does
not claim that a managed runtime erased every historical memory copy.

## Modes

The selected mode is project-local, non-secret configuration and is shown by
`kassiber operator status`:

- `manual` — each process supplies `--db-passphrase-fd` or prompts through the
  existing controlling-terminal path. No reusable broker lease and no implicit
  credential-store read.
- `brokered` — an operator authenticates once and the broker retains the
  passphrase only in memory for the chosen lease. This is the recommended mode
  for terminal/agent work sessions.
- `unattended` — explicit use of the existing CLI remembered-unlock item in the
  native OS credential store. It requires no continuing user presence and is
  never described as biometric or brokered authorization.

Existing projects with the old `cli_remembered_unlock` marker and no mode
setting are treated as legacy unattended configuration so an upgrade does not
silently strand automation. The next explicit mode change writes the new
setting. Selecting brokered mode disables remembered-unlock fallback for that
project.

## User commands

```bash
kassiber operator unlock --until-lock
kassiber operator unlock --duration 8h
kassiber operator status
kassiber operator lock
kassiber operator operation status <operation-id>
kassiber operator operation cancel <operation-id>
kassiber operator mode manual|brokered|unattended
```

An explicitly started session defaults to `--until-lock`. A duration accepts
human units such as `30m`, `8h`, and `2d`; there is no arbitrary short maximum.
The default cumulative grant is `accounting_decisions`, which includes `read`
and `operator`, so a real review session can resolve quarantine. A narrower
grant can be selected explicitly. `admin` is never a lease grant.

Interactive unlock may prompt on the controlling terminal. `--machine`,
`--non-interactive`, piped calls without an explicit secret fd, and ordinary
commands never open a surprise terminal or GUI prompt; they return the existing
`interaction_required` error with an operator command in the hint.

On macOS, an enrolled operator-specific Keychain item may be released after
Touch ID through the native LocalAuthentication path. Production-entitled
builds use an item-level current-biometry policy; preview builds report their
application-level fallback honestly. Windows Hello and Linux desktop
biometrics are not implemented. Password authorization is supported on macOS,
Linux, and Windows through the controlling-terminal/fd path.

## Endpoint and peer validation

The broker is one process per logged-in OS user.

- Linux uses an owner-only Unix socket and verifies `SO_PEERCRED.uid`.
- macOS uses an owner-only Unix socket and verifies `getpeereid()`.
- Windows uses a local named pipe whose protected DACL grants the current user
  only; after connection the broker resolves the client process token and
  compares its user SID with the broker SID. Remote pipe clients are rejected.

Runtime directories are created without following symlinks, must be owned by
the current user, and allow no group/other access. Socket/named-pipe creation is
the single-instance election. A failed election connects to the winner; stale
Unix socket files are removed only after an ownership/permission check and a
failed connect proves there is no listener.

Normal protocol records are length-prefixed JSON. Passphrases use a separate
length-prefixed secret frame bound to a one-use random challenge; secret bytes
are never members of a JSON object. Frames and command arguments are excluded
from logs.

## Canonical project identity and ownership

Callers select projects through the existing `--project` / `--data-root`
resolution. The broker resolves the database without trusting a display path,
rejects unsafe ownership, follows the final canonical target, and binds the
lease to a fingerprint of the resolved database file identity. Symlink aliases
therefore rendezvous with one lease. Replacing a database file produces a new
identity; moving the same file retains its file identity while the broker is
alive and is re-resolved on the next session.

The broker and desktop daemon take the same non-blocking project ownership
lock before retaining an unlocked runtime. Exactly one long-lived owner may
hold a project. Starting desktop-first or broker-first either rendezvous with
the existing supported owner or returns `project_in_use`; Kassiber never starts
a silent competing daemon. Short-lived broker child commands run only through
the owning project's serialized worker.

Workspace/profile/book selection remains explicit in each submitted command.
The child command re-parses and validates the original CLI scope inside the
selected project; queue state never supplies an implicit book.

## Capabilities

The authoritative exact-path registry is shared by CLI metadata, daemon
routing, and broker admission:

1. `read` — status, lists, searches, balances, report reads, and previews.
2. `operator` — imports, connection setup, sync, rates and journal processing,
   metadata maintenance, and ordinary exports.
3. `accounting_decisions` — quarantine resolution, reviewed custody/transfer
   decisions, exclusions, classification, and comparable interpretation
   changes.
4. `admin` — secret reveal/backup, passphrase and unlock-policy changes,
   destructive deletes/resets, credential management, and replication member
   or device administration.

The first three grants are cumulative. Unknown commands and daemon kinds fail
closed. Admin work always consumes a fresh, challenge-bound authorization for
one operation and does not upgrade the lease.

## Queue and operation semantics

Each project has one bounded FIFO queue (64 admitted operations). Different
projects have independent workers and can run concurrently. Capability and
lease validity are checked at admission and again immediately before dispatch.
Queue overflow returns retryable `operator_queue_full` with the limit; it never
pretends the operation was accepted.

Every accepted operation has an opaque id and one of these states:

- `queued`
- `running`
- `completed`
- `failed`
- `result_unknown`

The broker keeps the latest 256 operation/result records in RAM. Reconnecting
clients can query them or cancel queued work. A client disconnect does not
cancel accepted work. Running cancellation is advertised only where a command
has a cooperative cancellation contract; otherwise cancel returns an accurate
`not_cancellable` result.

Lock or expiry cancels queued work. A running child is allowed to finish or
roll back normally, after which the broker drops the retained passphrase. A
client waits by polling operation state without a fixed total timeout, so an
accepted mutation is never converted into a false terminal failure merely
because a UI deadline elapsed.

The operation id includes the broker generation. If a client asks a restarted
broker about an operation accepted by a dead generation, the answer is
`result_unknown` with a reconcile-before-retry hint. Kassiber does not claim
exactly-once mutation delivery across broker or worker crashes.

## Logging and audit

Unlock, lock, expiry, rejection, queue admission, dispatch, and crash state use
the existing bounded RAM-only log policy with insert-time secret-floor
redaction. No generic durable broker log or activity table is created. Durable
accounting audit continues to come from transaction edit history, custody
revisions, filed-report snapshots, and the domain-specific provenance written
by the operation itself.

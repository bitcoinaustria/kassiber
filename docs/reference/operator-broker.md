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
kassiber operator touch-id status
kassiber operator touch-id enroll
kassiber operator touch-id forget
```

An explicitly started session defaults to `--until-lock`. A duration accepts
human units such as `30m`, `8h`, and `2d`. The minimum is one minute; there is
no arbitrary maximum below the platform timestamp range. `--until-lock` is the
preferred deliberate work-session policy and has no timer.
The default cumulative grant is `accounting_decisions`, which includes `read`
and `operator`, so a real review session can resolve quarantine. A narrower
grant can be selected explicitly. `admin` is never a lease grant.

Interactive unlock may prompt on the controlling terminal. `--machine`,
`--non-interactive`, piped calls without an explicit secret fd, and ordinary
commands never open a surprise terminal or GUI prompt; they return the existing
`interaction_required` error with an operator command in the hint.

On macOS, an enrolled operator-specific Keychain item may authorize an unlock
after Touch ID through the signed desktop app's native LocalAuthentication
path. The broker starts the helper with a broker-created inherited output pipe.
Before any get, store, delete, or status action, the helper verifies that its
parent is the matching production-signed bundled Kassiber CLI sidecar. The
helper checks the live parent process with macOS Security.framework against a
fixed Developer ID Application requirement combining the exact
architecture-specific sidecar signing identifier and the helper's verified
TeamIdentifier. It also validates the bundle path and both static signatures.
It has no caller-selected socket or general
raw-secret "get" action and never returns the passphrase to the invoking CLI.
Production-entitled builds use an item-level current-biometry policy. Unlike
the separate desktop remembered-unlock feature, operator Touch ID has no
preview-build application-level fallback: unsigned/ad-hoc builds report it as
unavailable and use password authorization. Windows Hello and Linux desktop
biometrics are not implemented. Password authorization is supported on macOS,
Linux, and Windows through the controlling-terminal/fd path. Enrollment and
mode changes verify a fresh database passphrase; non-interactive callers use
the command's passphrase fd (or global `--operator-auth-fd` for a brokered
admin command) and never cause a surprise prompt. This fresh verification takes
a temporary project-owner lock, so mode selection and Touch ID enrollment work
from a clean locked state without creating a lease. That owner remains held
through the authenticated mode change or credential-store action, including
failure cleanup.

## Endpoint and peer validation

The broker is one process per logged-in OS user.

- Linux uses an owner-only Unix socket and verifies `SO_PEERCRED.uid`.
- macOS uses an owner-only Unix socket and verifies `getpeereid()`.
- Windows uses a local named pipe whose protected DACL grants the current user
  only; after connection the broker resolves the client process token and
  compares its user SID with the broker SID. Remote pipe clients are rejected,
  and client reads poll `PeekNamedPipe` against a monotonic deadline before
  entering `ReadFile`.

Runtime directories are created without following symlinks, must be owned by
the current user, and allow no group/other access. Named-pipe creation is the
single-instance election on Windows. Unix uses a separate owner-only,
non-blocking startup lock across stale probing, bind, and listen. A failed
election connects to the winner; stale Unix socket files are removed only by
the startup-lock holder after an ownership/permission check, and listener
shutdown unlinks only the exact socket inode it created.

On Linux, the broker watches logind's per-user state and the owner-only XDG
runtime directory, including the directory's original device/inode identity.
`closing`, `lingering`, or `offline` user state, removal, or replacement of the
runtime directory closes the broker and drops its leases. `online` is kept
alive because it is a valid logged-in but non-foreground user. macOS and
Windows logoff terminate the user process. On Linux systems exposing neither
logind nor an XDG runtime directory, broker startup fails with
`operator_session_lifetime_unavailable`; manual mode remains available rather
than claiming a logout guarantee the platform session cannot prove.

Normal protocol records are length-prefixed JSON. Passphrases use a separate
length-prefixed secret frame bound to a one-use random challenge; secret bytes
are never members of a JSON object. Frames and command arguments are excluded
from logs. The broker admits at most 64 connected clients at once and gives an
accepted Unix client 30 seconds to finish an individual inbound protocol
frame. Excess clients receive retryable `operator_client_limit` backpressure;
an idle or partial client therefore cannot consume threads without bound.

## Canonical project identity and ownership

Callers select projects through the existing `--project` / `--data-root`
resolution. The broker resolves the database without trusting a display path,
rejects unsafe ownership, follows the final canonical target, and binds the
lease to a fingerprint of the resolved database file identity. Symlink aliases
therefore rendezvous with one lease. Replacing a database file produces a new
identity; moving the same file retains its file identity while the broker is
alive and is re-resolved on the next session.

The admitted filesystem identity is re-resolved again immediately before
worker dispatch, again before child launch, and by the child immediately
before runtime/database bootstrap. Every initialized database also carries a
durable random `database_instance_id` in its own settings table. Unlock records
that identity from the opened connection, and broker children require the same
identity from the database they actually opened before schema migration or
command work. This applies to normal runtime bootstrap and direct SQLCipher
opens used by administrative commands. A queued operation is cancelled rather
than opening a replacement database at the admitted path.

At admission the broker also replaces every caller alias with the resolved
database parent's canonical path in both operation state and the child's
pinned global `--data-root`. No original symlink token survives into queued
argv, so retargeting that alias cannot redirect later no-bootstrap secret or
configuration work.

Unlock, fresh-auth continuations, retained lease state, scope refresh, and the
desktop owner's database open use that same canonical root. The desktop
project-switch path resolves and owns its target once, then verifies and opens
through that resolved root rather than returning to a catalog alias.

The broker and desktop daemon take the same non-blocking project ownership
locks before opening or retaining an unlocked runtime. One lock is keyed by
stable canonical file identity, one by each admitted canonical path, and one
is stored locally beside the project database. The global owner namespace is
stored below the OS account's persistent Kassiber runtime directory, is
derived from the account database rather than caller environment variables,
and deliberately ignores broker endpoint/runtime overrides. The
identity lock follows a moved/hardlinked file; path locks prevent a replacement
inode from becoming a concurrent project while the old lease remains
addressable for explicit lock/revocation. Exactly one long-lived owner may
hold a project. Starting desktop-first or broker-first either rendezvous with
the existing supported owner or returns `project_in_use`; Kassiber never starts
a silent competing daemon. Short-lived broker child commands run only through
the owning project's serialized worker.

Workspace/profile/book selection is made explicit at admission. A brokered
command whose CLI contract declares `--workspace` or `--profile` must supply
each declared scope flag; missing scope fails with `operator_scope_required`.
`context set` must supply at least one scope flag. The broker never borrows a
mutable lease-global context as an authorization default. The child re-parses
the pinned scope inside the selected project, so a later context change cannot
retarget queued work.

## Capabilities

The authoritative exact-path registry is shared by CLI metadata, daemon
routing, and broker admission:

1. `read` — status, lists, searches, balances, report reads, and previews.
2. `operator` — imports, connection setup, sync, live-rate acquisition,
   journal processing, note/tag maintenance, and ordinary exports.
3. `accounting_decisions` — quarantine resolution, reviewed custody/transfer
   decisions, exclusions, classification, and comparable interpretation
   changes.
4. `admin` — secret reveal/backup, passphrase and unlock-policy changes,
   destructive deletes/resets, credential management, and replication member
   or device administration.

The first three grants are cumulative. Unknown commands and daemon kinds fail
closed. Admin work always consumes a fresh, challenge-bound authorization for
one operation and does not upgrade the lease.

Backend connection setup is deliberately `operator` work, including creating
or updating an endpoint and submitting a new caller-supplied token. This lets a
granted work session connect and sync the project's accounting sources without
another prompt. It does not authorize reading a token already stored in the
project or deleting the backend: `backends reveal-token` and `backends delete`
remain `admin`. Because every same-user process may exercise the standing
grant, an operator lease should be granted only when routine import, sync, and
connection-configuration changes are acceptable for that session.

`backup import --install` is not brokerable. A queued destructive restore could
otherwise retarget either its destination or global project symlink after
validation, outside the inherited ownership handles. Backup import may still
decrypt and validate into a staging directory through a fresh-authorized
broker operation. Installing it requires locking the lease, selecting manual
mode, and explicitly running the restore for its destination.

## Queue and operation semantics

Each project has one bounded FIFO queue (64 admitted operations). Different
projects have independent workers and can run concurrently. Capability and
lease validity are checked at admission and again immediately before dispatch.
Fresh admin authorization has a 60-second monotonic dispatch lifetime; an admin
operation that waits longer is cancelled and its staged secrets are wiped.
Queue overflow returns retryable `operator_queue_full` with the limit; it never
pretends the operation was accepted.

Every accepted operation has an opaque id and one of these states:

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `result_unknown`

The broker keeps at most the latest 256 terminal operation/result records in
RAM, ordered by completion time and pruned on both admission and terminal
transition. It also keeps 1,024 bounded request-binding tombstones: replaying a
recent evicted operation id returns `result_unknown` and can never execute the
command again. Queued and running records do not consume the terminal-result
budget. Reconnecting clients can query retained results or cancel queued work.
A client disconnect does not cancel accepted work. Running cancellation is
advertised only where a command has a cooperative cancellation contract;
otherwise cancel returns an accurate `not_cancellable` result.

Lock or expiry immediately drains and cancels queued work, wipes its staged
secrets, and releases queue capacity. A running child is allowed to finish or
roll back normally, after which the broker drops the retained passphrase. A
client waits by polling operation state without a fixed total timeout, so an
accepted mutation is never converted into a false terminal failure merely
because a UI deadline elapsed.

Each operation runs in a short-lived direct CLI child. The lease passphrase and
any command-specific fd/stdin secrets cross inherited anonymous pipes, never
argv, environment variables, or JSON. Unix uses an explicit inherited-fd list;
Windows uses an explicit `STARTUPINFOEX` handle list and raw inherited handles.
The child eagerly drains and caches its lease pipe after project binding and
before command dispatch. The parent feeds all secret pipes on a dedicated
thread while concurrently draining child stdout/stderr, so platform pipe
capacity cannot deadlock a later command-specific secret handoff.
No-bootstrap children then authenticate and close the canonical database as a
preflight, enforcing the queued durable database identity before even a
credential-store-only or staged-backup handler can run.
The worker child inherits duplicate project-owner handles. If the broker dies,
the OS therefore keeps ownership exclusion in force until the orphan child
exits; a desktop or replacement broker cannot overlap it. Source installs
re-exec Python modules, while frozen one-file sidecars use hidden internal
entry modes rather than treating the bundled executable as a Python
interpreter. POSIX uses duplicated `flock` file descriptions; Windows opens
owner files with `CreateFileW` share mode zero and inherits duplicates of that
exclusive open-file reservation rather than relying on process-owned byte-range
locks.

The operation id includes the broker generation. If a client asks a restarted
broker about an operation accepted by a dead generation, the answer is
`result_unknown` with a reconcile-before-retry hint. Kassiber does not claim
exactly-once mutation delivery across broker or worker crashes.
Unproven nonzero exits from mutating/admin children are likewise
`result_unknown`; only read-only child failures are safely reported as
`failed`. Every brokered passphrase-rotation attempt revokes the old lease and
cancels its queued work after the child exits; this also covers a rekey that
succeeded before a later acknowledgement/invalidation failure, so status never
advertises an unusable stale secret.

Production Tauri and development-bridge calls likewise have no arbitrary
accepted-operation timeout. They wait for the exact request-id terminal record
or explicit process/transport failure; test-only supervisors may inject a
deadline to exercise late-response handling.

## Logging and audit

Unlock, lock, expiry, rejection, queue admission, dispatch, and crash state use
the existing bounded RAM-only log policy with insert-time secret-floor
redaction. No generic durable broker log or activity table is created. Durable
accounting audit continues to come from transaction edit history, custody
revisions, filed-report snapshots, and the domain-specific provenance written
by the operation itself.

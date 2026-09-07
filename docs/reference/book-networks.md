# Book network scope

A book is a profile inside a local project database. Settings → Bitcoin binds
that book to one environment: main, test, public signet, or a named regtest
instance. Bitcoin and Lightning share the Bitcoin domain; Liquid has its own
compatible domain. Signet does not imply a Liquid network. The binding is
immutable. Switching active books changes scope; it never relabels saved data.

Existing books start unbound. Local historical inspection remains available.
Accounting and report preparation reject mixed or contradictory histories.
External chain-analysis acquisition requires an explicit binding. A binding
neither authorizes network requests nor converts declared scope into ownership,
confirmed transactions, native observer evidence, or tax evidence.

## Review and bind

`networks inventory` reads wallet metadata, transactions, scoped acquired
observations and native observer/policy/output inventories. `networks plan`
previews a binding; `networks bind --plan-id …` recomputes the same recipe against
a consistent database snapshot. Any intervening source change invalidates the
preview. Unknown wallet scope requires an explicit `--declare-wallet ID`.
Contradictory or mixed histories cannot be overridden by a declaration.

For regtest, `--chain-instance-id UUID` identifies the user's particular local
chain history. Genesis and the word `regtest` cannot identify an instance.
After a reset, use a new instance and book; never reuse an old identity to
reinterpret historical transactions. Custom signet challenges are not configured
by this interface; the signet choice denotes the standard public network.

Wallet creation, updates, import batches, sync preflight/publication, replicated
row publication, journal construction and report preparation share the same
policy. Reviewed unknown connections use the book's routing default without
rewriting their historical config. Parallel sync results pin the environment
identity and cannot publish after a new binding. Explicit backend instance
assignments must match. Publicly shared graph caches need explicit matching
instance provenance before a regtest book can reuse them.

Bindings replicate as authored immutable scope. Conflicting peer bindings reject
replay instead of using last-writer-wins. Backend credentials and observer
ownership authority remain local.

## Split mixed history without changing the original

Settings → Bitcoin → Export selected connections, or `networks split-plan` and
`networks split`, produces a separate encrypted project backup. Choose the target
environment and connections, review the computed partition, then choose the
backup destination and encryption passphrase (CLI also accepts an age recipient).
The source project is never moved, pruned, or rewritten.

A profile configured for general accounting cannot be partitioned. Its ledger,
evidence and retained accounting decisions belong to the complete book and cannot
be copied safely by connection. The plan reports `accounting_partition_unsupported`;
export fails with `book_network_review_required`. Keep that book intact and use a
complete project backup when preserving its accounting history.

The partition retains original immutable row IDs. Authored relationships must be
closed within the selected connections: a custody component or other linked
record crossing the boundary blocks export. Attachments are checked against their
stored hashes. The target excludes unrelated profiles, backend and AI secrets,
derived journals, observer SDK caches and previously saved/filed reports. Native
observation provenance and selected wallet policy history remain evidence, not a
freshness claim. Source reports remain in the original project.

Restore the `.kassiber` file through the existing backup import flow into a new
project. It receives a fresh database identity and the reviewed immutable binding.
Reconnect its selected sources and rebuild journals before generating new reports.
Encrypted source projects retain inner database encryption as well as the outer
backup encryption. Export cannot overwrite an existing destination.

## Shared service

`core/book_network.py` owns inventory, preview/apply and the constant-size
`require_chain_domain` lookup. `ui.networks.binding` is the lightweight active-book
scope query. `ui.networks.inventory`, `.plan`, `.bind`, `.partition_plan` and
`.partition_export` power the desktop; the `networks` CLI uses those same services.
`core/book_network_migration.py` owns partition closure and the standard backup
artifact. Local analysis readers use `observation_matches_binding` before combining
physical observations from different stores.

# Recurring local-node acquisition

Recurring sources belong to one encrypted, unlocked book and its fixed Bitcoin domain. They reuse the configured Core connection and the existing redirect/proxy/TLS and `KASSIBER_NO_EGRESS` enforcement. This does not configure a node or enable a public fallback.

The local-only `sources plan` operation binds the source routing revision, database identity, book/domain revision, exact transaction or block range, lifetime request/response-byte budget and duration. `sources authorize` accepts only the unchanged plan. Authorization is independent of one-shot acquisition consent. Changing source routing, network or the book invalidates it. `sources revoke` fences new requests and in-flight publication while retaining acquired evidence.

The existing acquisition drawer contains **Keep local evidence up to date**. It shows compatible Core connections, one transaction or block history, and optional frequency/budget controls. Review displays the target, lifetime and quota before authorization. Status and revocation use the same drawer. No per-page network selector is added.

CLI example (after binding the book and configuring its own Core connection):

```sh
kassiber --machine chain-analysis sources plan --document '{"backend":"my-core","network":"main","mode":"blocks","start_height":0,"interval_seconds":300,"duration_days":30,"max_requests":1000000,"max_bytes":1000000000000}' --output source-plan.json
kassiber --machine chain-analysis sources authorize --plan @source-plan.json
kassiber --machine chain-analysis sources list
kassiber --machine chain-analysis sources run GRANT_ID
kassiber --machine chain-analysis sources revoke GRANT_ID --expected-revision 1
```

`run` queues one bounded batch; execution requires the running unlocked daemon. Scheduled batches catch up historical ranges, then wait the approved interval. A finite end height remains observed for later chain changes. This is not an OS service and does not run while Kassiber is closed or the book is locked. On-device agents have the same typed plan/authorize/list/run/revoke operations; source artifacts are unavailable to remote models. Authorization requires once-only consent with server-recomputed effects.

Each request durably reserves one request and the maximum response size before transport. Completed responses refund unused bytes. Interrupted requests retain their reservation, so crashes cannot increase the allowance. A response may be refused when the remaining quota cannot cover its maximum size. No unbounded retry or implicit quota renewal exists. Execution leases permit one publisher per grant; revocation and changed scope are checked between requests and again before commits.

Block ingestion uses Core `getblockhash`, `getblockcount` and raw `getblock`, so it does not require `txindex`. It resumes from a durable cursor and verifies header hash, transaction parsing and Merkle commitment before retaining sanitized structure. Raw witness stacks and block bytes are discarded. Core remains the consensus validator; these observations neither establish wallet ownership nor grant accounting authority. A pruned block remains unavailable; the scan never skips the gap and claims complete history.

Each transaction occurrence has a block-hash/position key, preserving historical duplicate TXIDs. Inputs reference the preceding canonical occurrence when available. A partial-range start leaves earlier inputs unknown. Reorg reconciliation withholds uncertain source membership, walks back in bounded steps, then reactivates the surviving prefix and ingests replacement blocks. Historical records remain stored. Independent regtest instances use explicit book identities; genesis alone is not an instance identifier.

Confirmation depth comes from a verified tip and retained block membership, with an observation timestamp. Tip changes update a small shared overlay instead of rewriting every historical transaction. Chain Analysis and watches read the same values; an uncertain reorg cannot supply a current confirmation count.

The derived tables and execution permissions are not replicated. Existing saved cases and PDFs remain frozen. Source assertions enter the shared incremental index without entering wallet imports, journals or owned-output inventory.

Verification lives in `tests/test_chain_analysis_backfill.py`: durable quota, scope/source fencing, revocation during transport, crash-conservative reservations, wrong genesis, repeated TXIDs, input occurrence resolution, Merkle mismatch, multi-step reorg and CLI/agent boundaries. `tests/test_chain_analysis_backfill_integration.py` exercises a real encrypted book, shared worker, index and watch inbox, including lock cancellation and confirmation thresholds. The disposable Core lane in `tests/integration/test_live_chain_analysis.py` also covers bounded range resumption and a real node reorg.

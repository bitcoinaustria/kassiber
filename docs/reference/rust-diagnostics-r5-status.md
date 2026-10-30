# R5 diagnostics acceptance

Base: `50f67616` (integrated `t3code/rust-backend-r5c`). Local implementation
branch: `t3code/r5-slice-diagnostics`. Python product code is unchanged.

## Boundary

| Surface | Native implementation | Parity evidence |
| --- | --- | --- |
| `diagnostics collect` | `crates/diagnostics`, CLI family adapter, admitted book projection | Ordered report and CLI bytes, saved report bytes/modes, planted wallet/backend/transaction fields |
| `--diagnostics-out PATH\|auto` | Shared CLI failure boundary, before book close | Automatic/custom exports, best-effort write failures, original error envelopes, success/usage paths without accidental exports |
| Diagnostics export failures | Original I/O exception type and unhandled state retained | Collect/save failures with and without `--debug`, error/report byte comparison |
| Diagnostics daemon kinds | None exist in the Python reference | No kind, desktop or AI allowlist added |
| Redaction | CPython Unicode regex semantics, ordered fourteen rules, key tokens, recursive shapes | 29 text, 135 key, 140 structured-detail, four argument cases; planted secrets under every sensitive field |
| Aggregate state | Read-only SQL and runtime backend projection | Golden Python state with effective/ineffective reviewed terms and 146 syntactic locality vectors |

The approved differences are listed precisely in
[rust-shell.md](rust-shell.md#diagnostics-service-slice-r5): native compiler and
implementation identity, linked SQLite version, an empty report stack, and an
empty requested debug string. Debug is null without `--debug`, even for an
unhandled CLI failure. Report/error schemas, ordering and types are retained.
Tests freeze the clock rather than hiding generated timestamps. Explicit
normalization allowlists have regressions proving unrelated fields remain
observable. Native frames are never fabricated or serialized as paths.

No socket, DNS, provider probe, raw configuration export or disk logging is
added. Only the user-requested report reaches disk. The existing network
source/dependency guard reviews the compiler-version build call and pure IP
enum matches; real connector forms are still rejected.

## Verification

Verification uses Rust 1.99.1, the locked Python 3.11 environment and offline
Cargo. Disposable books and export paths are used throughout; HOME is not
overridden by the runner commands.

The checked-in catalog has 98 mutations, killed by named native tests. It
covers every text rule and sensitive key, Unicode/shape handling, native stack
and debug refusal/presence, effective manual review selection, credential and
wallet-config projection, and backend locality. The final state refinements
also reran their three affected mutation entries.

The final focused hybrid run passed 17 tests and recorded 25 Rust-served calls
with zero Python fallbacks. Every candidate oracle invocation asserts its
served-by record is Rust and its report identifies itself as Rust.

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `diagnostics.collect` | 8 | 0 |
| `rates.range` (global error-report flag) | 6 | 0 |
| `init` | 3 | 0 |
| usage validation | 1 | 0 |
| workspace/profile/backend/wallet creation, CSV import, note set, status | 7 | 0 |

Final checks: workspace tests **1,187 passed**, preflight **57 passed** (57
subtests), native diagnostics tests **11 passed**, mutation catalog **98/98
killed**, formatting and both workspace/test-harness clippy checks passed.
The broader shell/hybrid/runner/entrypoint and silent-payment redaction run
passed **140 tests and 284 subtests**. The classifier update and subsequent
check passed.

Commands used:

```sh
MATURIN_NO_INSTALL_RUST=1 mise exec rust@1.99.1 uv@latest -- uv sync --locked --python 3.11
mise exec rust@1.99.1 -- cargo fmt --all -- --check
mise exec rust@1.99.1 -- cargo clippy --workspace --all-targets --offline --locked -- -D warnings
mise exec rust@1.99.1 -- cargo test --workspace --offline --locked
mise exec rust@1.99.1 -- cargo build -p kassiber --features test-harness --offline --locked
mise exec rust@1.99.1 uv@latest -- uv run --locked python crates/diagnostics/scripts/mutation_check.py
mise exec rust@1.99.1 uv@latest -- uv run --locked python -m tests.rust_port.classify_tests update
mise exec rust@1.99.1 uv@latest -- uv run --locked python scripts/python_test_shards.py run preflight -- -q
```

The initial sandboxed workspace attempt could not bind existing chain-test
loopback listeners. Workspace validation therefore runs outside that sandbox;
diagnostics collection itself needs no sockets.

Native macOS/Windows identity/filesystem runs, interactive TTY scenarios and
live regtest are not run in this Linux slice. The non-Unix identity fallback
has no OS-release source; Windows identity parity remains a platform gate.
The full Python/frontend quality gate was not run; this local slice is not
claimed push-ready. No push, PR or deployment is part of this work.

## Python reference bugs and repros

Distinct generated timestamps collapse to the same automatic report filename.
The timestamp redactor runs before filename construction, yielding
`kassiber-diagnostics--timestamp-.json`; a subsequent export overwrites it.
Python also writes directly with `Path.write_text`, retaining the inode and
existing permissions; it does not atomically replace or force a private mode.
Under the verification process's ordinary umask, a new report was `0644`.
Rust preserves these behaviors rather than changing the reference.

Reproduce entirely on a disposable root:

```python
from pathlib import Path
from tempfile import TemporaryDirectory
from kassiber.diagnostics import save_public_diagnostics_report

with TemporaryDirectory() as directory:
    data_root = str(Path(directory) / "data")
    first = save_public_diagnostics_report(
        {"generated_at": "2026-10-29T01:02:03Z", "marker": "first"},
        target="auto", data_root=data_root,
    )
    second = save_public_diagnostics_report(
        {"generated_at": "2026-10-30T04:05:06Z", "marker": "second"},
        target="auto", data_root=data_root,
    )
    assert first.path == second.path
    assert first.path.name == "kassiber-diagnostics--timestamp-.json"
    assert '"second"' in first.path.read_text()
```

No Python fix is included.

### R6 integration reference alignment

Automatic report filenames retain `generated_at`, replace unsafe filename characters, and use exclusive creation with `-2`, `-3` suffixes on collisions. Explicit custom targets retain overwrite behavior. Python/Rust parity covers repeated automatic exports.

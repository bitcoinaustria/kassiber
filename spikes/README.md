# R0 backend feasibility spikes

Research baseline: `8709d4304c3cd7b76396020947ac4b6bd66e7ebe`, 2026-10-26,
Linux x86_64. All four packages are standalone Cargo workspaces with their
own lockfiles. Product code and the root workspace are unchanged. Evidence
contains synthetic data only; databases and CLI state use disposable roots.
Only dependency downloads and the explicitly exercised local Secret Service
were allowed external I/O. The PDF baseline additionally denies DNS/connect
calls and enables the product no-egress switch.

## 1. SQLCipher — feasible with conditions

[`sqlcipher/run.py`](sqlcipher/run.py) calls Kassiber's real
`open_encrypted`, `rekey_connection` and plaintext migration, and the
[`Rust executable`](sqlcipher/src/main.rs) uses rusqlite **0.40.1** /
libsqlite3-sys **0.38.2**, with `bundled-sqlcipher-vendored-openssl` and
`backup`. [Recorded run](evidence/sqlcipher.txt).

| Runtime | SQLCipher | Embedded SQLite | Existing product journal rule |
| --- | --- | --- | --- |
| Python 3.11.17, sqlcipher3 0.6.2 | 4.12.0 community | 3.51.1 | DELETE / FULL |
| Rust 1.99.1, rusqlite 0.40.1 | 4.14.0 community | 3.51.3 | WAL / NORMAL |
| Python stdlib sqlite3 (plaintext) | none | 3.53.1 | WAL / NORMAL |

Passed both directions: create/open/read/write; rekey in **each** language
and reopen in the other; SQLCipher export from plaintext; metadata
`user_version=21` / `auto_vacuum=FULL`; encrypted backup via both backup APIs
while source and a writer remain open; integrity/cipher-integrity; wrong-key
rejection. Passphrases include quote, newline and emoji. Empty/NUL keys are
refused. Key first, then compatibility 4, KDF 256000, page size 4096, then
schema verification, exactly as Python. Rust also creates the complete current
`db.SCHEMA` (100 tables including internal objects), which Python opens.
JSON extract/each/group/valid, window functions, UPSERT and RETURNING pass.
No page-format incompatibility was found.

The [WAL-reset fix](https://www.sqlite.org/wal.html#walresetbug) requires
SQLite >=3.51.3, or the 3.50.7 / 3.44.6 backports. Preserve the exact
`db.py` predicate, query **the current connection's** version and fail
closed to DELETE/FULL on unknown/vendor strings. Do not choose SQLCipher
journaling from the unrelated stdlib SQLite version.

Feature choice: vendored OpenSSL **3.6.3** builds a portable static crypto
dependency on this Linux host (`ldd` confirms no libcrypto/libssl dependency),
appropriate for AppImage and Windows where host crypto availability varies.
It adds build prerequisites (C toolchain, make/Perl; MSVC/Perl and any
OpenSSL assembly tooling on Windows), size and crypto-update responsibility.
For macOS, prefer target-specific `bundled-sqlcipher` with its native
Security/CoreFoundation crypto branch when OPENSSL_* overrides are absent;
it avoids another bundled library. Alternatively keep vendored OpenSSL for
one pinned provider across platforms. The [upstream build rules](https://docs.rs/crate/libsqlite3-sys/0.38.2/source/build.rs)
are the authority; **neither macOS variant nor Windows was built here**.

Conditions before product adoption: run these same file round trips on both
macOS architectures and Windows; assert crypto linkage in packaged artifacts;
carry all connection/temp/mmap/busy settings and project ownership locks;
port migration's verified sibling/rollback/rename protocol. The Rust export
spike proves contents/metadata and retains plaintext; it is not the full
crash-safe file replacement implementation. Hot backup here is a live-open
snapshot with a committed second-handle write, not a power-loss or concurrent
checkpoint stress test. Keep R3's durability and rollback gates.

```sh
mise exec rust@1.99.1 -- cargo build --locked --manifest-path spikes/sqlcipher/Cargo.toml
mise exec rust@1.99.1 -- cargo test --locked --manifest-path spikes/sqlcipher/Cargo.toml
uv run --locked python spikes/sqlcipher/run.py
```

## 2. Native keyring — feasible with conditions

[Compatibility table and exact modifiers](keyring/COMPATIBILITY.md) cover
Python keyring **25.7.0**, the **4.0.0** Rust facade's store selection, and
the desktop's actual keyring-core / platform-store **1.0.0** dependencies.
The spike pins those desktop versions rather than silently adopting newer
store behavior. [Live Linux evidence](evidence/keyring.txt),
[Python Windows algorithm vectors](evidence/windows-formats.txt).

Linux passed a real Python-created item → Rust read/update → Python read,
with unchanged D-Bus item identity, application/schema attributes and label;
the one synthetic item was deleted. A session bus and unlocked default
Secret Service collection were available. This is real service evidence,
not the in-memory keyring backend. Windows algorithm evidence uses an
explicitly identified WinCred stand-in on Linux; Mac/Windows native tests
remain unexecuted.

Conditions: Windows must handle **bare service plus account@service** for
Python remembered-unlock, preserve persistence and use UTF-16 password APIs.
The desktop's existing Windows **AI** items instead use **account.service,
Local persistence and raw UTF-8**. Current Python delegates native AI storage
to that Rust desktop channel; it does not write those items with Python
keyring. Keep the namespace-specific formats separate. On Linux omit
`target`, retain existing extra attributes and handle collection/scheme
ambiguity explicitly. On macOS keep classic generic-password service/account
attributes and require the new executable's native ACL authorization when
necessary. Never broaden ACLs or implicit unlock enrollment.

```sh
mise exec rust@1.99.1 -- cargo build --locked --manifest-path spikes/keyring/Cargo.toml
uv run --locked python spikes/keyring/run.py
uv run --locked python spikes/keyring/windows_formats.py
```

## 3. macOS signing / Touch ID — feasible with conditions

[Concrete design and owner checklist](macos-identity/DESIGN.md) distinguish
broker build identity, the helper's exact Identifier/Team/CDHash, live
launcher/spawned-helper PID checks, and the helper's live-parent Developer
ID requirement. [Linux identity-vector evidence](evidence/macos-identity.txt).

A Rust Mach-O can satisfy the current helper checks with **no weaker
requirements** by retaining the architecture-specific sidecar basename,
signing identifier, same-bundle location and team. Keep the signed outer
app/helper, provisioning profile, exact Keychain entitlements, ready/go gate,
inherited secret pipes and current-biometry policy. The sidecar remains
unentitled and hardened-runtime signed. Port the broker's own PID checks;
having the desktop helper check the Rust parent alone is insufficient.

One Mach-O removes PyInstaller one-dir collection/signing complexity, not
Developer ID/notarization or source-provenance requirements. Raw binaries
cannot carry stapled notary tickets; retain the app-bearing CLI distribution
for Touch ID and offline first launch. Mac signing, real remembered-item
ACL access and hardware Touch ID **must still pass on a Mac** before R1 is
accepted. Linux vectors prove only portable serialization and origin rules.

```sh
mise exec rust@1.99.1 -- cargo test --locked --manifest-path spikes/macos-identity/Cargo.toml
mise exec rust@1.99.1 -- cargo build --locked --manifest-path spikes/macos-identity/Cargo.toml
uv run --locked python spikes/macos-identity/run.py
```

## 4. ReportLab replacement — feasible with conditions

[`summary-pdf`](summary-pdf/) uses Typst / typst-pdf / typst-layout
**0.15.1** as libraries, an embedded template, JSON data through `sys.inputs`,
native selectable chart labels and vector primitives, plus an in-memory SVG
donut. Its `World` only serves the embedded template, bundled fonts and that
SVG; no system font scan, filesystem imports, package resolver or runtime
network fetch. The standard library alone cannot provide this layout/PDF
engine. No Typst package or executable is needed at runtime.

The Python baseline runs the real CLI's init, workspace, profile, custom
wallet, four-row CSV import, journals process and summary PDF export. It
captures exactly the writer's report model and fixes `generated_at` for the
comparison. No tax/journal decision is recomputed in the renderer.

| Evidence | Python ReportLab 4.5.0 | Rust Typst 0.15.1 |
| --- | --- | --- |
| [PDF](summary-pdf/evidence/python.pdf) / [PDF](summary-pdf/evidence/rust.pdf) | 3 A4 pages; 9,955 bytes | 2 A4 pages; 72,147 bytes |
| Text / row / chart comparison | 79 semantic strings, including 56 chart labels, present in both | Same; word-count differences only page furniture and repeated activity-table header |
| Same captured input, repeated runs | Identical bytes | Identical bytes across separate processes |
| PDF format | 1.4, untagged, invariant metadata date 2000 | 1.7, tagged, explicit metadata date 2026-05-01 |
| Local rendering sample | ~100 ms first / 25 ms warm call | ~22–24 ms process render |

[Comparison JSON](summary-pdf/evidence/comparison.json) and extracted
[Python text](summary-pdf/evidence/python.txt) / [Rust text](summary-pdf/evidence/rust.txt)
retain the measurements. These are one synthetic input, not a performance
benchmark or acceptance of changed pagination. Typst's compact cells fit the
activity/appendix on page 2; Python splits activity onto page 3. Vera changes
text metrics; chart legends use colored text rather than ReportLab's swatches,
axis/marker styling and spacing differ; subject/keywords metadata still need
porting. All snapshot/KPI/integrity/movement/composition/activity/disposal/
appendix sections and the line, donut, PnL and paired-flow charts are included.

Fonts: bundled unmodified Vera regular/bold from the locked ReportLab wheel,
with its full Bitstream license. That wheel lacks `VeraMono.ttf`, so today's
`register_fonts` actually falls back to Helvetica/Helvetica-Bold/Courier for
all three names. Typst explicitly uses Vera and a bundled Liberation Sans
regular fallback (SIL OFL 1.1) for the missing `→` glyph. Font licenses and
provenance are under [fonts](summary-pdf/fonts/). No system fonts are needed
to reproduce the Rust PDF; matching the current accidental Helvetica fallback
would require a separately agreed font choice.

Size/build: stripped release executable **40,202,040 bytes** (~38.34 MiB),
including fonts and renderer; an empty stripped Rust executable here is
331,920 bytes. The ~39.87 MB difference is a standalone upper-bound reference,
not the marginal size in a shared backend. Fresh dependency release build
with two jobs and offline crates took **286.5 s**, then ~2.5 s incremental
to link the report. These exclude download time and are machine-specific;
production LTO/features may change them.

Conditions: accept/adjust layout and metadata deliberately; test long/Unicode
labels, dense/empty/negative/multi-asset reports, font coverage and all four
product reports. Use the backend's Python-compatible decimal type for the
production port: this isolated rendering experiment uses `rust_decimal`
only for display formatting and float geometry only for drawing. Determinism
here pins fonts, template, compiler, report timestamp, PDF date and World
clock. Derive those dates from the explicit report context in production;
do not keep this fixture's hard-coded date or let package/font discovery
introduce runtime I/O. It is not a claim of byte parity with ReportLab.

Source-funds: [the current module](../kassiber/core/source_funds_diagram.py)
builds one frozen flow/ring model into one ReportLab Drawing, then native
PDF and desktop SVG; font rewriting supports the browser. Preserve that
single geometry model in Rust. Prefer direct deterministic SVG from it for
the desktop, with the same paths consumed as an in-memory SVG image in
Typst PDF (as this spike's donut demonstrates). Escape labels, prohibit
external href/fonts/images, and keep detail thresholds, truncation, rings,
colors, unsupported/unknown states and numeric authority unchanged. SVG text
can become outlines in PDF; if selectable diagram labels are required,
emit the same geometry with native Typst text overlays or a native vector
PDF layer. Typst layout alone is not a desktop SVG API or an accounting model.

```sh
mise exec rust@1.99.1 -- cargo build --release --locked --manifest-path spikes/summary-pdf/Cargo.toml -j 2
uv run --locked python spikes/summary-pdf/baseline.py
uv run --locked python spikes/summary-pdf/compare.py
```

## Plan 21 follow-up

- Record these Linux proofs as R0 evidence; keep native Mac/Windows item
  and packaging checks and a signed hardware Touch ID flow as R1/R3 gates.
- Pin/test SQLCipher **and its embedded SQLite**; preserve the conditional
  WAL gate across Python/Rust transition and rollback. Add per-platform crypto
  feature/linkage decisions and migration/backup crash tests.
- Add explicit Windows remembered-unlock vs desktop-AI format adapters,
  Linux default-collection/alternate-scheme cases, macOS ACL reauthorization,
  and the guarded AppImage build-origin rule to R1 acceptance vectors.
- Keep the current macOS sidecar filename/identifier and outer app identity
  even though the backend is one Mach-O. Plan packaging/provenance changes
  without treating raw-binary notarization as offline distribution parity.
- Budget ~40 MB and cold Typst compile cost; require text/table/chart and
  approved layout comparisons with embedded fonts and an offline World.
  R0 demonstrates feasibility, not finished report parity or Mac validation.
- Reconcile the plan's stated 91-table inventory with actual current SCHEMA
  results (100 including internal objects); use the contract inventory's
  counting convention rather than blindly replacing the number.

Setup used the requested `mise install rust@1.99.1` and
`MATURIN_NO_INSTALL_RUST=1 mise exec rust@1.77.0 uv@latest -- uv sync --locked --python 3.11`.
Cargo caches were placed at `/tmp/kassiber-r0-cargo` via `mise exec
rust@1.99.1 -- env CARGO_HOME=/tmp/kassiber-r0-cargo cargo …`; repeat commands
above can use that same cache. Python scripts use only locked project packages
and the standard library; `pdftotext`/`pdfinfo` 26.10.0 are external inspection
tools, not new product dependencies. No push/PR or full product quality gate
is claimed; this change only adds isolated research packages and evidence.

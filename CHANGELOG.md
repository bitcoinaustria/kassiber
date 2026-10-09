# Changelog

## 0.22.78

### Breaking Changes

- Remove device sync; older books drop their replication tables on open while keeping every authored record ([#628](https://github.com/bitcoinaustria/kassiber/pull/628)).
- Remove the general ledger; books with archived ledger rows keep them until an explicit purge ([#629](https://github.com/bitcoinaustria/kassiber/pull/629)).

### Added

- Add native encrypted backup and safe desktop restore ([#563](https://github.com/bitcoinaustria/kassiber/pull/563)).
- Track chain changes affecting saved and filed reports ([#564](https://github.com/bitcoinaustria/kassiber/pull/564)).
- Preview and confirm acquisition classifications with exact accounting effects ([#565](https://github.com/bitcoinaustria/kassiber/pull/565)).
- Explain capital-gains results from retained engine and source evidence ([#566](https://github.com/bitcoinaustria/kassiber/pull/566)).
- Restore intra-wallet chain lineage for source-of-funds and let it assemble itself ([#569](https://github.com/bitcoinaustria/kassiber/pull/569)).
- Explain quarantine causes, roots and next actions ([#576](https://github.com/bitcoinaustria/kassiber/pull/576)).
- Show why transactions are quarantined and what to provide ([#579](https://github.com/bitcoinaustria/kassiber/pull/579)).
- Unify the macOS title bar with the shell and size the window to the screen ([#581](https://github.com/bitcoinaustria/kassiber/pull/581)).
- Remember the side nav, fold it in narrow windows, and add history shortcuts ([#582](https://github.com/bitcoinaustria/kassiber/pull/582)).
- Open the command palette as a launcher, and keep it closed while locked ([#583](https://github.com/bitcoinaustria/kassiber/pull/583)).
- Run brokered commands in the caller's context and make agent outcomes machine-readable ([#588](https://github.com/bitcoinaustria/kassiber/pull/588)).
- Add `kassiber mcp`, a stateless, read-only MCP server for external agents ([#589](https://github.com/bitcoinaustria/kassiber/pull/589)).
- Add an off-by-default desktop switch for external agents (MCP) ([#593](https://github.com/bitcoinaustria/kassiber/pull/593)).
- Let the desktop unlock its encrypted book for the agents you allow ([#596](https://github.com/bitcoinaustria/kassiber/pull/596)).
- Switch books from a picker on the title-bar crumb ([#602](https://github.com/bitcoinaustria/kassiber/pull/602)).
- Rebuild Reconcile as an input-and-results workbench ([#603](https://github.com/bitcoinaustria/kassiber/pull/603)).
- Name unsupported formats in Reconcile and match Lightning invoices ([#604](https://github.com/bitcoinaustria/kassiber/pull/604)).
- Match silent-payment addresses in Reconcile ([#605](https://github.com/bitcoinaustria/kassiber/pull/605)).
- Stream Reconcile's on-chain verify with progress and stop ([#606](https://github.com/bitcoinaustria/kassiber/pull/606)).
- Add a glass ribbon 3D view to the expanded transaction graph ([#609](https://github.com/bitcoinaustria/kassiber/pull/609)).
- Add an offline mode that blocks every outbound connection ([#614](https://github.com/bitcoinaustria/kassiber/pull/614)).
- Cache connection checks and offer an opt-in automatic check ([#615](https://github.com/bitcoinaustria/kassiber/pull/615)).
- Draw the transaction graph as mempool's bowtie in glass ([#617](https://github.com/bitcoinaustria/kassiber/pull/617)).
- Show a wallet's coins as glass blocks above its UTXO table ([#618](https://github.com/bitcoinaustria/kassiber/pull/618)).
- Fill a synced wallet's transaction graphs after the sync ([#619](https://github.com/bitcoinaustria/kassiber/pull/619)).
- Walk the book's history from the graph and list coins below it ([#620](https://github.com/bitcoinaustria/kassiber/pull/620)).
- Give the transaction sheet a wide workspace and a back trail ([#621](https://github.com/bitcoinaustria/kassiber/pull/621)).
- Measure one large wallet's sync and journal step on regtest ([#623](https://github.com/bitcoinaustria/kassiber/pull/623)).
- Make BTCPay a first-class, multi-store balance source ([#626](https://github.com/bitcoinaustria/kassiber/pull/626)).
- Plan extensions and treat outside pull requests as suggestions ([#627](https://github.com/bitcoinaustria/kassiber/pull/627)).
- Sign release manifests with an OpenSSH release key ([#631](https://github.com/bitcoinaustria/kassiber/pull/631)).

### Fixed

- Fix cross-book caches, transaction detail edits, and sync edge cases ([#560](https://github.com/bitcoinaustria/kassiber/pull/560)).
- Fix custody evidence consistency and book initialization ([#561](https://github.com/bitcoinaustria/kassiber/pull/561)).
- Fix LND payment settlement dates and refresh existing imports ([#562](https://github.com/bitcoinaustria/kassiber/pull/562)).
- Preserve native evidence and stop overstating source-of-funds claims ([#567](https://github.com/bitcoinaustria/kassiber/pull/567)).
- Trim agent guidance and repository docs ([#568](https://github.com/bitcoinaustria/kassiber/pull/568)).
- Mount UI tests for source-of-funds auto-assembly, and re-arm it when a draft clears ([#571](https://github.com/bitcoinaustria/kassiber/pull/571)).
- Load assembly inputs once per history assembly ([#572](https://github.com/bitcoinaustria/kassiber/pull/572)).
- Give the attested-refresh fixture an asset ([#573](https://github.com/bitcoinaustria/kassiber/pull/573)).
- Restore fail-closed custody holds and fix the RP2 carry lookup ([#575](https://github.com/bitcoinaustria/kassiber/pull/575)).
- Rebuild journals after every user-triggered sync and import ([#577](https://github.com/bitcoinaustria/kassiber/pull/577)).
- Stop presenting incomplete cost basis as final ([#578](https://github.com/bitcoinaustria/kassiber/pull/578)).
- Keep the dev bridge on the preview's launch book when an import is cleared ([#585](https://github.com/bitcoinaustria/kassiber/pull/585)).
- Frame the lock and setup screens like the shell, and drop the block-field art ([#586](https://github.com/bitcoinaustria/kassiber/pull/586)).
- Fix encrypted-book agent paths: credential migration, lock contention, unlock guidance ([#587](https://github.com/bitcoinaustria/kassiber/pull/587)).
- Give every modal the command palette's treatment ([#592](https://github.com/bitcoinaustria/kassiber/pull/592)).
- Show own Liquid amounts and size confidential strands honestly in the transaction graph ([#594](https://github.com/bitcoinaustria/kassiber/pull/594)).
- Bind the operator broker to its build so several Kassiber builds can share a machine ([#595](https://github.com/bitcoinaustria/kassiber/pull/595)).
- Put page spacing and surface corners on shared layout tokens ([#597](https://github.com/bitcoinaustria/kassiber/pull/597)).
- Stop repeating the page name below the title bar ([#598](https://github.com/bitcoinaustria/kassiber/pull/598)).
- Draw every page card with the shared surfaces ([#599](https://github.com/bitcoinaustria/kassiber/pull/599)).
- Lay out settings as rows and use the app's own controls ([#600](https://github.com/bitcoinaustria/kassiber/pull/600)).
- Rebalance light mode and make focus rings ink ([#601](https://github.com/bitcoinaustria/kassiber/pull/601)).
- Frost the overview chart where cost basis is incomplete ([#611](https://github.com/bitcoinaustria/kassiber/pull/611)).
- Remove the Live data row from the side nav ([#612](https://github.com/bitcoinaustria/kassiber/pull/612)).
- Rebuild the book switcher as grouped book lists ([#613](https://github.com/bitcoinaustria/kassiber/pull/613)).
- Stop re-emitting unchanged BDK transactions and repeating the wallet's scripts in every row ([#624](https://github.com/bitcoinaustria/kassiber/pull/624)).
- Preserve Apple notarization progress ([`2a3eeb89`](https://github.com/bitcoinaustria/kassiber/commit/2a3eeb8985479d91b48db22feed6dda574325aa3)).
- Move macOS release signing into protected CI ([`d85e0dd6`](https://github.com/bitcoinaustria/kassiber/commit/d85e0dd6ea77f311b0dbb57623d72a8211b70ab8)).
- Resume pending macOS notarization without resubmitting ([`8835b221`](https://github.com/bitcoinaustria/kassiber/commit/8835b22113672607040789a691d717814bd2adf9)).
- Use reviewed changelog sections for release notes ([`ef0c3ffb`](https://github.com/bitcoinaustria/kassiber/commit/ef0c3ffb26bff18135ecf152aa4e590da6c0483a)).
- Show one line per source in Add connection ([`1c66d396`](https://github.com/bitcoinaustria/kassiber/commit/1c66d3965c6df9d7d62db01f9ea59cdb25a12b0d)).
- Stop the app's scroll panes from rubber-banding ([`55e1ce1b`](https://github.com/bitcoinaustria/kassiber/commit/55e1ce1bc9288b48090bcc9f7448439e3078f6e9)).
- Stop dialogs, swap sheets and onboarding from rubber-banding ([`12d9bb1f`](https://github.com/bitcoinaustria/kassiber/commit/12d9bb1f9655471e85fddb17de9160c525f2b8a4)).

## 0.22.77

0.22.77 was never published; its changes first shipped in 0.22.78, except the
general-ledger entries (#545, #546, #550), which were removed before publication
([#629](https://github.com/bitcoinaustria/kassiber/pull/629)).

### Added

- Import CoinTracking and Blockpit history with automatic reconciliation ([#527](https://github.com/bitcoinaustria/kassiber/pull/527)).
- Encrypted general accounting and Austrian workpapers through the CLI ([#545](https://github.com/bitcoinaustria/kassiber/pull/545)).
- Accounting tasks through agents with explicit local consent ([#546](https://github.com/bitcoinaustria/kassiber/pull/546)).
- Selected financial AI assistance with explicit disclosure grants ([#550](https://github.com/bitcoinaustria/kassiber/pull/550)).
- Local chain investigations and guided source-of-funds reports ([#551](https://github.com/bitcoinaustria/kassiber/pull/551)).
- Incremental local chain observation index ([#553](https://github.com/bitcoinaustria/kassiber/pull/553)).
- Encrypted local evidence watches and a change inbox ([#554](https://github.com/bitcoinaustria/kassiber/pull/554)).
- Bounded recurring Bitcoin Core acquisition and resumable block history ([#555](https://github.com/bitcoinaustria/kassiber/pull/555)).
- Developer ID signing and automated macOS notarization tooling ([#547](https://github.com/bitcoinaustria/kassiber/pull/547)).
- Unpublished macOS notarization candidates ([#557](https://github.com/bitcoinaustria/kassiber/pull/557)).

### Fixed

- Unify custody and source-of-funds review across agents, CLI and desktop ([#543](https://github.com/bitcoinaustria/kassiber/pull/543)).
- Reject non-fiat cash legs in generic Bitcoin ledger imports ([#544](https://github.com/bitcoinaustria/kassiber/pull/544)).
- Book compensation as acquisitions with configurable basis pools ([#542](https://github.com/bitcoinaustria/kassiber/pull/542)).
- Update cryptography and frontend dependencies ([#548](https://github.com/bitcoinaustria/kassiber/pull/548)).
- Update the RP2 dependency ([#549](https://github.com/bitcoinaustria/kassiber/pull/549)).
- Bind books to immutable chain networks and safely split mixed history ([#552](https://github.com/bitcoinaustria/kassiber/pull/552)).
- Fix Linux repository publishing on Bash 3.2 ([#556](https://github.com/bitcoinaustria/kassiber/pull/556)).
- Create new book schemas in one transaction ([#558](https://github.com/bitcoinaustria/kassiber/pull/558)).
- Serialize refresh execution and preserve failure diagnostics ([#559](https://github.com/bitcoinaustria/kassiber/pull/559)).
- Reject negative fees before tax normalization ([`4f198208`](https://github.com/bitcoinaustria/kassiber/commit/4f198208c325bfd1dd8a7b0c3e37f653cb2aba57)).
- Cover negative fee evidence rows by anchor ([`4956f76b`](https://github.com/bitcoinaustria/kassiber/commit/4956f76bb34b78632d1a2394161e1912d25d1a0d)).
- Keep transaction details in one sheet while resolving ([`3714fbd5`](https://github.com/bitcoinaustria/kassiber/commit/3714fbd5973688f4bdce8fc811aa21f8a172027a)).
- Update package versions to 0.22.77 ([`0ba2eb6c`](https://github.com/bitcoinaustria/kassiber/commit/0ba2eb6c9a360a4f5ef821b093981e343dd3a780)).

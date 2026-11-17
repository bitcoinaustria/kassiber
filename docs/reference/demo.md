# Demo book

Kassiber ships a sample book so anyone can click through the whole suite
without connecting a wallet: wallets on several layers, about 1,100
transactions from 2019 to 2026, reviewed custody, source-of-funds cases,
filed reports and evidence, priced with synthetic regtest rates. It is a
disposable copy, kept apart from the user's own books.

## Starting it

- Desktop: **Try the demo** on Welcome, or **Try the demo book** in the book
  switcher. A banner then marks every screen, with **Leave demo** and a menu
  holding **Reset demo** and **Connect an agent**.
- CLI: `kassiber projects demo [--path DIR]`, which prints the new copy's
  `data_root`. Use it with `--data-root` like any explicit book.

Each start, and each **Reset demo**, writes a fresh copy. **Leave demo**
reopens the book that was open before and deletes the copy.

## Where it lives

`kassiber projects demo` writes a new directory
`<base>/<UTC stamp>-<hex>/` with the usual layout (`config/settings.json`,
`data/kassiber.sqlite3`, `attachments/`, `exports/`) and the marker
`config/demo.json`. `<base>` is `--path`, or `demo/` under the state root.
The desktop passes `KASSIBER_DEMO_ROOT` as `--path` when it is set, so tests
and development runs stay out of the real state root.

After writing the copy, the command removes earlier copies under `<base>`:
only directories that carry the marker, never through a symlink. A copy that
is still open on Windows stays until the next start. A `--path` that holds
anything else is refused (`demo_path_not_empty`), so the command never deletes
the user's files.

The demo is not registered in `config/projects.json`. It never appears among
the user's books and cannot be selected into one. `--data-root` and
`--project` are refused on `projects demo`.

The command then opens the copy like any book, which runs the schema
migrations, so a payload written by an older build opens on a newer schema. It
also builds the local analysis index (about 3 seconds), so the first
investigation does not pay for it. The copy has a fresh
`database_instance_id`. It grows from 15 MB to about 62 MB once the index is
built.

## Privacy

Creating and opening the demo contacts nothing. The bundled book has no sync
backend. The payload build removed every backend row and the default backend,
set backend setup to manual, and tombstoned the built-in public defaults, so
none of them is added on open. It also removed the generator's wallet
`backend` and `source_file` keys. As a result:

- wallet sync and **Check connections** fail closed with
  `backend_not_configured`;
- the desktop's automatic graph lookup from a wallet's backend reports
  `wallet_backend_lookup_unavailable`, and the explicit **Look up on-chain**
  has no backend either;
- AI and external agents work as in any book, behind the same
  acknowledgement, consent and plan gates. Nothing is sent until the user
  configures a provider and asks.

`crates/kassiber/tests/demo_book.rs` proves this with product egress allowed
and the strict test observer refusing every connection, loopback included.

A failed sync is recorded like in any book: that wallet's source turns
blocking, and exports report it until a sync succeeds. In the demo,
**Reset demo** starts over instead.

Source-of-funds snapshots in the sample keep explorer links to the
generator's loopback explorer (`127.0.0.1`). The snapshots are hashed
evidence and stay unchanged. Kassiber opens such a link only when it is
clicked, in the system browser.

### Preferences

The demo daemon runs with its own `offline-mode.json`, `agent-access.json`
and `wallet-backend-lookup.json` under the copy's `config/`, through
`KASSIBER_OFFLINE_PREFERENCE_FILE`, `KASSIBER_AGENT_ACCESS_FILE` and
`KASSIBER_WALLET_BACKEND_LOOKUP_FILE`. `projects demo` seeds all three from
the user's current files (an absent file stays absent, so its default
applies; `preferences_seeded` says which were copied) and returns the
variables as `preference_environment`, which the desktop passes to the demo
daemon and must use for its own writes of these files. The user's offline
choice therefore
carries into the demo, while turning offline mode off, enabling external
agents, or turning on Privacy's "Fill in transaction details from the
wallet's backend" inside the demo never changes the user's own settings. Window
and UI preferences (theme, language) stay shared. The desktop never stores
the demo as the open book or remembers a demo route.

### AI providers

Providers, their keys and chats are stored in each book's database, so the
demo starts with only the seeded local defaults. A hosted model is added in
the demo's **Settings → Assistant**, exactly as in a real book. Providers are
not copied from the user's books, so no key reference or remote
acknowledgement crosses books.

## External agents (MCP)

Turn on **External agents** in the demo's Settings → Assistant. The command
shown there includes the demo's preference files, so the agent reads the
demo's grant:

```json
{
  "mcpServers": {
    "kassiber": {
      "command": "kassiber",
      "args": ["--data-root", "<copy>/data", "mcp", "serve"],
      "env": {
        "KASSIBER_AGENT_ACCESS_FILE": "<copy>/config/agent-access.json",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": "<copy>/config/offline-mode.json",
        "KASSIBER_WALLET_BACKEND_LOOKUP_FILE": "<copy>/config/wallet-backend-lookup.json"
      }
    }
  }
}
```

`projects demo` prints the same values as `mcp_server`. Without the `env`
entries the server reads the user's own agent setting. A reset creates a new
copy at a new path, so register the server again afterwards.

## The payload

The book is compiled into the `kassiber` binary
(`crates/kassiber/assets/demo-book.sql.gz`, about 1.8 MB): a gzip'd SQL dump
that `flate2`, already a dependency, unpacks. Because CLI, daemon and MCP
server are one binary, every package carries it without a separate resource
file. The analysis index and AI tables are stripped; journals, rates, filed
reports and transaction graph references (`transaction_graph_cache`) are
kept. Attachment files travel in a staging table (`demo_payload_files`) that
`projects demo` extracts and drops.

| | size |
| --- | --- |
| payload in the binary | 1.78 MB |
| restored database | 15 MB |
| after the index build | 62 MB |

### Regenerating the payload

1. Build the demo book with the regtest harness
   (`scripts/integration-harness.sh demo-full`, see
   [testing](testing.md)).
2. While its backends are still reachable, run wallet syncs until the graph
   follow-up has nothing left to fill. Each sync fills at most 50
   transactions and 250 prevouts into `transaction_graph_cache`.
   Graphs then show outside inputs' amounts offline. The current payload has
   no cached graph rows: the graphs draw every output, and every input whose
   parent is a stored transaction, but outside inputs have no amount.
3. `scripts/build-demo-payload.sh <book dir>` strips and compresses the book,
   relabels the regtest stack's invented live quote (which the harness
   stores as a `mempool` price) as `regtest-demo-synthetic`, like the rest
   of the synthetic rate continuation, refuses one that still holds backend
   credentials, generator paths or a `mempool`-labelled rate, and writes the
   payload. It uses `sqlite3`, `gzip` and `python3 -I` (standard
   library only, for the Python-format wallet JSON); none of them is a
   product dependency.

The payload is source data, not a test seed; `scripts/check-no-test-tape.sh`
is unaffected.

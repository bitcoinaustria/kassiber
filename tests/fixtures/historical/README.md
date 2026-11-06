# Historical database fixtures

These fixtures are databases created by the Kassiber code at the named commit,
not current databases with selected columns removed afterward.

| Fixture | Source commit | `kassiber/db.py` SHA-256 | Decoded database SHA-256 |
| --- | --- | --- | --- |
| `pre_432_5d232097.sqlite3.gz.b64` | `5d232097` (parent boundary of PR #432) | `b06b46c9d92660cee335f8dfb4d2947b7516fa8fef29bc1bc10ce139b7206dd4` | `26edd1582dcc18e85f4807b0eb9d7ccff4aad01baf1cd705ac546d40ac238c45` |
| `pre_435_16b7bdc1.sqlite3.gz.b64` | `16b7bdc1` (first-parent boundary before PR #435) | `b06b46c9d92660cee335f8dfb4d2947b7516fa8fef29bc1bc10ce139b7206dd4` | `e33c7035cad1b516e9aa2ee9115eca27e3a6e33502435ae4be5086d18d07d72b` |

The two source commits have byte-identical database schema code. They remain
separate fixtures because the PR boundaries are independent compatibility
promises.

Each of the two custody-overhaul databases above contains:

- one explicit, reviewed manual transfer pair;
- two concurrent authored-active custody components from different replicas;
- a 10 BTC outbound and a 9.9 BTC return across an unobserved Whirlpool gap;
- preserved Samourai parent/postmix metadata, with deliberately missing
  descriptor material;
- processed journal/report-derived rows but no filed-report assertion;
- two replication events and one unresolved custody conflict (device sync
  was later removed; current code drops them on open).

The fixture builder is intentionally run with the historical source tree first
on `PYTHONPATH`. It refuses to overwrite an existing output directory. After
generation, gzip and base64 encode `kassiber.sqlite3` to obtain the committed
text fixture. The compatibility test decodes into a temporary directory and
lets the current `open_db` perform the real upgrade.

The fixtures are plaintext SQLite because they test schema/data migration.
SQLCipher encryption, passphrase rotation, and plaintext-to-encrypted migration
have separate tests; cipher-page compatibility is not claimed by these files.


## Partial payout pricing correction

`pre_partial_payout_ca9a672f.sqlite3.gz.b64` was built and processed with
`ca9a672f`, before the partial payout projection fix. Source `db.py` SHA-256:
`4c44164d25de749da4abe619cffd9887251f1d277639a572e5f7778a50ed5a2e`; decoded database SHA-256:
`1d5647d6122b362ea05db518a1dda3b614b0c9742bffe5b697abdf7d5d62ad20`.

The disposable generic EUR book has a 200,000-msat buy at 1,000,000 EUR/BTC,
a 100,000-msat outbound at 2,000,000 EUR/BTC and a taxable direct payout covering
40,000 msat with reviewed value 10 EUR. Historical processing stored 20 EUR
proceeds by pricing both slices at the full payout value. The upgrade must
refuse reports until rebuilding; corrected proceeds are 11.2 EUR (10 + 1.2).
It was generated with the reviewer's `upg_a.py`, changing only its scratch/source
paths, against an archived historical Python tree. It contains no real data.

## Main #654 book for the Rust main sync

`pre_sync_bae01537b.sqlite3.gz.b64` was written by main at `bae01537b`
(PR #654, the base of the Rust port before main sync 1) with
`build_pre_sync_fixture.py`. Source `db.py` SHA-256:
`4c44164d25de749da4abe619cffd9887251f1d277639a572e5f7778a50ed5a2e`; decoded database SHA-256:
`fab38e9852e8f61b32390742b4e78aefe3f843634b2a814b0f20cae4493ee0a5`.

The disposable Austrian EUR book holds three wallets with buys, a sale,
interest and an owned transfer, processed journals, an AI provider and two
custody AI audit rows under #654's narrower consent CHECK. Opening it runs
every schema change main added after #654 (the accepted coarse price, the
provider `agent_access` column, `ai_provider_checks`, the widened consent
CHECK) and the semantic migrations the Rust store ports. Python and Rust must
produce the same schema, rows and migration ledger from it.

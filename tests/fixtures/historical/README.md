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

Each database contains:

- one explicit, reviewed manual transfer pair;
- two concurrent authored-active custody components from different replicas;
- a 10 BTC outbound and a 9.9 BTC return across an unobserved Whirlpool gap;
- preserved Samourai parent/postmix metadata, with deliberately missing
  descriptor material;
- processed journal/report-derived rows but no filed-report assertion;
- two replication events and one unresolved custody conflict.

The fixture builder is intentionally run with the historical source tree first
on `PYTHONPATH`. It refuses to overwrite an existing output directory. After
generation, gzip and base64 encode `kassiber.sqlite3` to obtain the committed
text fixture. The compatibility test decodes into a temporary directory and
lets the current `open_db` perform the real upgrade.

The fixtures are plaintext SQLite because they test schema/data migration.
SQLCipher encryption, passphrase rotation, and plaintext-to-encrypted migration
have separate tests; cipher-page compatibility is not claimed by these files.


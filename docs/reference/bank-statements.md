# Bank statements as source-of-funds evidence

Banks ask where the money for a Bitcoin purchase came from. Kassiber reads
the statement files a user downloads from their own bank and lets them attach
the line that paid for a purchase as source-of-funds evidence. Files only:
no FinTS/HBCI, no PSD2, no aggregator, no account at any provider, and
importing, matching, reviewing and reporting open no network connection.

A bank line is evidence of what the bank booked. It never becomes a
transaction, journal row, balance or ledger entry, and it grants no
ownership, basis or tax fact. Each import is an `external_documents` row of
type `statement` with the file as its attachment. The bank tables
(`crates/store/fixtures/optional-schema.json`) are created by a book's first
statement import, inside that import's transaction.

## Formats

- **CAMT.053** (`camt.053.001.02` to `.08`): booked `Ntry` entries with
  amount, `CdtDbtInd`, booking and value date, `AcctSvcrRef`, and from a
  single `TxDtls` the counterparty name and IBAN, remittance and
  `EndToEndId`. Pending (`PDNG`, `INFO`, `FUTR`) entries are skipped and
  counted. A batch entry (several `TxDtls`) stays one line with its booked
  amount and no counterparty; its parts are never split out. DTDs and other
  namespaces are refused.
- **CSV**: one generic reader. It reads UTF-8 (BOM optional) or Latin-1,
  tries `;`, tab and `,`, and finds the header within the first 40 rows by
  German and English names (Buchungsdatum/Booking date, Valuta, Betrag/Amount
  or Soll/Haben/Debit/Credit, Soll/Haben marks, Währung/Currency or
  `Betrag (EUR)`, Empfänger/Counterparty, IBAN, Verwendungszweck/Reference,
  Bankreferenz). When a file names its columns differently, a column map
  names them, by header or 1-based number:

  ```json
  {"booking_date": "Tag", "amount": "Wert", "currency": 6,
   "remittance": ["Zweck 1", "Zweck 2"], "decimal": ",", "date_format": "DD.MM.YYYY"}
  ```

`--format auto` reads XML as CAMT and anything else as CSV. MT940 is refused
by name (`bank_statement_mt940`); export CAMT.053 or CSV instead.

## Refused, never guessed

Each refusal names the row, column number or field and never quotes the file.

- A decimal mark no row decides (`1.000` alone), a day/month order no row
  decides (`03/04/2025`), malformed grouping (`1,2,3`), a currency symbol,
  space or apostrophe grouping, or two columns that could be the same field.
  A column map settles each.
- A currency that is not one of Kassiber's fiat codes
  (`kassiber_transactions::FIAT_CODES`; crypto assets and stablecoins
  included), currencies that disagree within a row (column, code in the
  amount, amount header, `--currency`), or no currency at all.
- A line without an own account: CAMT names it per statement; a CSV needs
  `--account` (IBAN, number or a label) or a mapped `account` column.
- A CSV whose quoted field is never closed (it would swallow the rows after
  it), and a row with an amount whose date cell starts like a date but is
  not one (`2025-0x-02`); blank dates and footer labels are skipped.

## Storage and re-import

Amounts are exact signed decimal text (outgoing is negative). Counterparty
IBANs are stored masked only; the own account is stored in full, like
addresses. The original file name is never stored or echoed.

The same bytes for the same account and parsing configuration record nothing
(`already_imported`). The same bytes read with another configuration are
refused (`bank_statement_reparsed`); roll that import back first. Lines are
grouped by account, booking date, amount, currency, direction and bank
reference: a line equal to a stored one on every field is skipped only when
the bank's reference or end-to-end id identifies it; without one it is
refused (`bank_statement_line_ambiguous`), and a line whose group holds only
different lines is refused naming the field (`bank_statement_line_conflict`).
Within one file, a line carrying the bank's reference that repeats an earlier
line of the file is that same booking (counted as existing); a second version
of it is refused the same way.

`documents rollback-statement` plans first and with `--confirm` deletes the
import's lines, document and file; it refuses while a line is accepted
evidence. "Wipe everything" deletes the bank tables' rows with their
documents; "Reset book data" keeps them. A network partition of a book
holding statements is refused (`bank_statements_partition`).

## Matching and review

A suggestion pairs one outgoing line with one inbound BTC `buy` when all hold:
the same fiat currency, the bank amount equal to the purchase's exact fiat
value (`fiat_value_exact`, or the exact rate times the amount), a booking
date from 10 days before to 1 day after the purchase, and no other candidate
on either side. Reasons name the currency, amount, day gap and a curated
exchange in the counterparty or remittance text. Everything else is listed
in `unmatched` with its reason: `no_exact_amount` (fees, partial or split
payments) or `ambiguous` (two candidates). Purchases without an exact value
are listed separately. Accepted and rejected pairs leave the pool.

Accepting (a suggestion, or any explicit lines and purchases) reuses a
purchase's reviewed `fiat_purchase` source together with its reviewed link,
or creates the source and links it as a `manual_source` (`method`
`bank_statement`), and records an `accepted` decision; the purchase keeps one
allocation. The desktop accepts a suggestion by its `suggestion_id` (the
suggestion's `id`): the daemon refuses it (`conflict`) unless the pair is
still exactly that suggestion, so a match shown as exact and unique never
lands as an explicit pairing after the purchase or the statements changed. Generated text names only dates, amounts, a curated exchange and
the account's short form (`••3201`). Rejecting records `rejected`; on an
accepted pair it withdraws the line from the source's evidence and keeps the
source. The source-of-funds report lists accepted lines as `bank_evidence`
on their source: date, amount and currency always, the bank from `minimal`,
counterparty, masked IBANs and bank reference in `standard` and `full`;
never remittance text, never the statement file.

## Privacy

Outputs show bank free text (names, remittance, references, bank name) with
every IBAN in it masked (compact or in groups of four, valid check digits).
Other account numbers inside free text are not recognized. The assistant
and MCP tools see masked IBANs, a counterparty only as a curated exchange
name, and no remittance text, bank references, previews or column plans
(`ai_projection`, `ai_project_bank_evidence`); accept and reject go through
the usual plan and consent gates. Bank lines never enter diagnostics, logs
or support bundles. Statement files live in the attachments directory,
outside SQLCipher, like every attachment.

## Surfaces

```sh
kassiber documents import-statement --file statement.xml [--format auto|camt053|csv] \
  [--account IBAN-or-label] [--column-map '{...}'|@map.json] [--currency EUR] [--label LABEL] [--dry-run]
kassiber documents statement-imports
kassiber documents rollback-statement --import IMPORT_ID [--confirm]
kassiber source-funds bank-lines list [--direction out|in|all] [--document DOC]
kassiber source-funds bank-lines suggest [--transaction TX]
kassiber source-funds bank-lines accept|reject --line LINE [--line ...] --transaction TX [--transaction ...]
```

Daemon kinds of the same names (`ui.documents.import_statement`,
`.statement_imports`, `.rollback_statement`, `ui.source_funds.bank_lines.list`,
`.suggest`, `.accept`, `.reject`) take the same arguments through one parser
(`Command::from_args`).

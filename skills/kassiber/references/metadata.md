# Metadata

Use this reference for notes, tags, exclusions, BIP329 labels, and attachments.

## Metadata records

Inspect transaction-facing metadata:

```bash
kassiber metadata records list --limit 50
kassiber metadata records list --wallet satoshi-liquid --has-note
kassiber metadata records get --transaction <transaction-id>
```

Useful filters:

- `--wallet`
- `--tag`
- `--has-note`
- `--no-note`
- `--excluded`
- `--included`
- `--start`
- `--end`
- `--cursor`
- `--limit`

## Notes

```bash
kassiber metadata records note set --transaction <transaction-id> --note "Reviewed against BTCPay payout"
kassiber metadata records note clear --transaction <transaction-id>
```

Use `--reason` on note, tag, and exclusion mutations when the change needs an
auditor-facing explanation. The CLI records `source=cli`; desktop saves record
`source=gui`; approved assistant tool mutations record `source=ai_tool`.

## Tags

```bash
kassiber metadata records tag add --transaction <transaction-id> --tag reviewed
kassiber metadata records tag remove --transaction <transaction-id> --tag reviewed
```

## Exclusions

```bash
kassiber metadata records excluded set --transaction <transaction-id>
kassiber metadata records excluded clear --transaction <transaction-id>
```

After exclusions or other review metadata that change reporting meaning, re-run:

```bash
kassiber journals process
```

## Edit history and Activity

Every real metadata edit writes append-only history rows in the same local
SQLite transaction as the metadata change. No-op saves do not create history.
Revert creates a new forward edit; it never rewrites old rows.

```bash
kassiber metadata records history list --transaction <transaction-id>
kassiber metadata records history activity --source ai_tool --field-family pricing
kassiber metadata records history activity --transaction <transaction-id> --limit 25
kassiber metadata records history stale
kassiber metadata records history revert --event-id <event-id> --field note --reason "Undo mistaken note"
```

History rows store normalized machine values and render human summaries,
including grouped pricing changes, tag added/removed diffs, source badges, and
redacted sensitive values. Activity supports date, source, field-family, wallet,
transaction, pricing-only, and AI-only filters.

## BIP329

```bash
kassiber metadata bip329 preview --file /path/to/labels.jsonl
kassiber metadata bip329 import --file /path/to/labels.jsonl
kassiber metadata bip329 list
kassiber metadata bip329 export --mode stored --file /path/to/export.jsonl
kassiber metadata bip329 export --mode synthesized --wallet satoshi-liquid --file /path/to/wallet-labels.jsonl
```

Import is profile-wide and stores every valid BIP329 row. Only exact transaction
matches become Kassiber tags by default; ambiguous rows are preserved but skipped
unless `--apply-ambiguous` is used after reviewing the preview. Wallet-scoped
export is conservative and emits only rows Kassiber can tie deterministically to
that wallet.

## Attachments

```bash
kassiber attachments add --transaction <transaction-id> --file /path/to/document.pdf
kassiber attachments add --transaction <transaction-id> --url https://example.invalid/proof
kassiber attachments list --transaction <transaction-id>
kassiber attachments rename <attachment-id> --label "Accountant approval"
kassiber attachments verify
kassiber attachments gc
```

Attachments are managed local files or literal URL references. Kassiber does not fetch and index URL attachments. Unnamed URL labels are derived for display and can be renamed without changing the URL target.

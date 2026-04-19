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

## BIP329

```bash
kassiber metadata bip329 import --wallet satoshi-liquid --file /path/to/labels.jsonl
kassiber metadata bip329 list --wallet satoshi-liquid
kassiber metadata bip329 export --wallet satoshi-liquid --file /path/to/export.jsonl
```

## Attachments

```bash
kassiber attachments add --transaction <transaction-id> --file /path/to/document.pdf
kassiber attachments add --transaction <transaction-id> --url https://example.invalid/proof
kassiber attachments list --transaction <transaction-id>
kassiber attachments verify
kassiber attachments gc
```

Attachments are managed local files or literal URL references. Kassiber does not fetch and index URL attachments.

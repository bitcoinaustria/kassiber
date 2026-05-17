# Screenshots

Drop screenshots, animated GIFs, and recorded terminal sessions here.

## Naming

Use kebab-case with a context prefix so files group by surface:

- `desktop-overview.png`
- `desktop-reports-austrian-e1kv.png`
- `desktop-swap-matching.png`
- `desktop-source-of-funds.png`
- `desktop-assistant.png`
- `cli-quickstart.gif`
- `cli-transfers-suggest.gif`
- `cli-diagnostics-collect.png`

## Embedding

Reference from any markdown file with a relative path. From the repo root,
that's `docs/assets/screenshots/<file>`; from `docs/`, it's
`assets/screenshots/<file>`.

```markdown
![Overview screen — wallets, balances, and recent activity](docs/assets/screenshots/desktop-overview.png)
```

## Where to put them

- **README.md** — a short "Screenshots" section between `## Highlights` and
  `## Install`, with 2–4 hero images (desktop overview, reports, swap
  matching, an animated CLI quick start).
- **docs/quickstart.md** — inline near the workflow step they illustrate
  (e.g. a `transfers suggest` GIF in the transfer-pairing section).
- **docs/reference/desktop.md** — per-screen screenshots beside the screen
  description.

## Recording

- CLI animated GIFs: [vhs](https://github.com/charmbracelet/vhs),
  [asciinema-agg](https://github.com/asciinema/agg), or
  [terminalizer](https://github.com/faressoft/terminalizer).
- Desktop captures: use the OS screenshot tool. Crop tightly and prefer
  retina/high-DPI captures so the result is sharp on GitHub's rendering.
- Sanitize before committing: never include real wallet labels, addresses,
  txids, descriptors, or backend hostnames. Use the demo data root from
  `scripts/generate-source-funds-demo-report.py` or a throwaway profile.

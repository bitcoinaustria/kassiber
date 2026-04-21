# UI Review Gallery

This directory is for local, generated screenshot review output while comparing
Claude Design mockups against the QML shell.

The checked-in source of truth is:

- the scripts under [`scripts/`](../../../scripts/)
- frozen design references under `docs/design/phase-<n>/<screen>/refs/`

Optional local input folders for the gallery loop:

- `docs/design/review-reference/` for named reference screenshots like `overview-data.png`
- any Claude export directory passed with `--jsx-root`

Do not commit machine-local gallery output from this directory. The generated
files are ignored on purpose:

- `generated/*.png`
- `reference/*`
- `index.html`
- `manifest.json`

Example:

```bash
uv run python scripts/build_ui_review_gallery.py \
  --reference-images-dir docs/design/review-reference \
  --jsx-root /path/to/claude-export \
  --html-export /path/to/export.html
```

# Bundled report fonts

The Typst reports embed these fonts; the renderer never scans system fonts.
DM Sans and JetBrains Mono are the desktop's typefaces. Text and amounts use
DM Sans (amounts with its tabular figures, `tnum`); identifiers such as
transaction ids and hashes use JetBrains Mono. Liberation Sans is the glyph
fallback for characters DM Sans lacks (Greek, Cyrillic, ₿); characters no
bundled font covers are replaced by U+FFFD before layout, so nothing renders
as a missing glyph.

| File | Face | Source | SHA-256 |
| --- | --- | --- | --- |
| `DMSans-Regular.ttf` | DM Sans 400, version 4.004 | [googlefonts/dm-fonts](https://github.com/googlefonts/dm-fonts) at commit `4412393b7d2de9fe7a92064c2dce9b5af5d7fd26`, `Sans/fonts/ttf/DMSans-Regular.ttf` | `a95a338942aa357145427be7c963ff691f9fe859c3512d9c0483e1914d57a208` |
| `DMSans-Medium.ttf` | DM Sans 500, version 4.004 | same commit, `Sans/fonts/ttf/DMSans-Medium.ttf` | `26af1a05d62906ffc3316ee6b4cbaf7ae86a1e607c5b9d89c64ec37f85063fbf` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono 400, version 2.304 | [JetBrains/JetBrainsMono release v2.304](https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip) (archive SHA-256 `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf`), `fonts/ttf/JetBrainsMono-Regular.ttf` | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| `LiberationSans-Regular.ttf` | Liberation Sans 2.1.5 | Arch Linux `ttf-liberation 2.1.5-2`, unmodified upstream release | `baccc64becc3eb7d104b7c84d99f5314a0a1f896e2b3ea6c2f22fc08d2003bee` |

The upstream files are bundled unmodified and only the weights the reports
use. They were downloaded once, on 2026-11-04, with the owner's approval.

Licenses: SIL Open Font License 1.1 for all three families, in
[`LICENSE-DMSans`](LICENSE-DMSans) (from `google/fonts`, `ofl/dmsans/OFL.txt`),
[`LICENSE-JetBrainsMono`](LICENSE-JetBrainsMono) (the release archive's
`OFL.txt`) and [`LICENSE-Liberation`](LICENSE-Liberation).

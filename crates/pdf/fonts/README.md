# Bundled report fonts

The Typst reports embed these fonts; the renderer never scans system fonts.
DM Sans and JetBrains Mono are the desktop's typefaces. Liberation is only a
glyph fallback for characters outside the Latin subsets (₿, →).

| File | Face | Source | SHA-256 |
| --- | --- | --- | --- |
| `DMSans-Regular.ttf` | DM Sans 400 | `@fontsource/dm-sans` 5.3.0, `files/dm-sans-latin-400-normal.woff` (SHA-256 `38702875fe179437d200e7ba6f7fab4d497ffe48117ef332029b2081751186ad`) | `c6d271083968a7366a7c6202bb1ec6b73d6e9f9c1a9b66ef6a1ad567af9bfb10` |
| `DMSans-Medium.ttf` | DM Sans 500 | `@fontsource/dm-sans` 5.3.0, `files/dm-sans-latin-500-normal.woff` (`7deb957e430b780ee3272d801af4f9f2211b1330ec40a312a8f25309eacb4034`) | `39951dbe12b0afea371ec3a978c8c8dd309babd5e33a7cbfbbbcfd66ddb8dbd3` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono 400 | `@fontsource/jetbrains-mono` 5.2.8, `files/jetbrains-mono-latin-400-normal.woff` (`658b9ee07b0249cf910a8d6f3d812d46c962bae81d7379a0355b1db35c61586f`) | `51de3d64b566c1190865f2ed92ab7cbd637fb7854caff9a3651544ba71cfd9f1` |
| `LiberationSans-Regular.ttf` | Liberation Sans 2.1.5 | Arch Linux `ttf-liberation 2.1.5-2`, unmodified upstream release | `baccc64becc3eb7d104b7c84d99f5314a0a1f896e2b3ea6c2f22fc08d2003bee` |
| `LiberationMono-Regular.ttf` | Liberation Mono 2.1.5 | as above | `47ed5b5fcfe6b3c9228937b05de9c769f5fa55b777d539af0f65172f0a24c90b` |

The DM Sans and JetBrains Mono files are the `@fontsource` Latin subsets
(Google Fonts builds of the upstream projects) converted from WOFF 1.0 to
TrueType by `../scripts/woff_to_ttf.py`. WOFF 1.0 only zlib-compresses each
table, so the conversion restores the original font byte for byte; no glyph,
metric or name is changed. These subsets carry no `tnum` feature and DM Sans
has proportional digits, so reports set figures in JetBrains Mono.

Licenses: SIL Open Font License 1.1 for all three families, in
[`LICENSE-DMSans`](LICENSE-DMSans), [`LICENSE-JetBrainsMono`](LICENSE-JetBrainsMono)
and [`LICENSE-Liberation`](LICENSE-Liberation).

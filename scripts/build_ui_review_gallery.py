#!/usr/bin/env python3
from __future__ import annotations

import argparse
import html
import json
import shutil
import subprocess
import sys
from pathlib import Path


SCENE_SOURCES = {
    "welcome": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/welcome.jsx",
        "html_scene": "welcome",
    },
    "overview-empty": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/overview.jsx",
        "html_scene": "overview",
    },
    "overview-data": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/overview.jsx",
        "html_scene": "overview-full",
    },
    "transactions": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/transactions.jsx",
        "html_scene": "transactions",
    },
    "tax": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/tax.jsx",
        "html_scene": "reports",
    },
    "connection-detail": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/connections.jsx",
        "html_scene": "connection-detail",
    },
    "settings": {
        "jsx": "/Users/dev/Github/kassiber/temp/screens/settings.jsx",
        "html_scene": "settings",
    },
}

IMAGE_EXTENSIONS = {".png", ".jpg", ".jpeg", ".webp"}


def _copy_reference_images(reference_dir: Path, output_dir: Path) -> dict[str, str]:
    if not reference_dir.exists():
        return {}

    copied: dict[str, str] = {}
    reference_output = output_dir / "reference"
    reference_output.mkdir(parents=True, exist_ok=True)
    for path in sorted(reference_dir.rglob("*")):
        if not path.is_file() or path.suffix.lower() not in IMAGE_EXTENSIONS:
            continue
        scene_id = path.stem.strip().lower().replace("_", "-").replace(" ", "-")
        if scene_id not in SCENE_SOURCES:
            continue
        target = reference_output / f"{scene_id}{path.suffix.lower()}"
        shutil.copy2(path, target)
        copied[scene_id] = str(target.relative_to(output_dir))
    return copied


def _capture_previews(captures_dir: Path, scenes: list[str]) -> None:
    cmd = [
        sys.executable,
        "scripts/capture_ui_previews.py",
        "--output-dir",
        str(captures_dir),
        "--scenes",
        *scenes,
    ]
    subprocess.run(cmd, check=True)


def _render_gallery(
    output_dir: Path,
    captures_dir: Path,
    reference_images: dict[str, str],
    scenes: list[str],
) -> None:
    rows = []
    manifest = []
    for scene in scenes:
        source = SCENE_SOURCES.get(scene, {})
        capture_path = captures_dir / f"{scene}.png"
        capture_rel = str(capture_path.relative_to(output_dir)) if capture_path.exists() else ""
        reference_rel = reference_images.get(scene, "")
        jsx = source.get("jsx", "")
        html_scene = source.get("html_scene", "")
        manifest.append(
            {
                "scene": scene,
                "capture": capture_rel,
                "reference_image": reference_rel,
                "jsx_source": jsx,
                "html_export": "/Users/dev/Downloads/Kassiber.html",
                "html_scene": html_scene,
            }
        )

        reference_panel = (
            f'<img src="{html.escape(reference_rel)}" alt="{html.escape(scene)} reference" />'
            if reference_rel
            else '<div class="empty">Drop a matching screenshot here later to compare this scene automatically.</div>'
        )
        capture_panel = (
            f'<img src="{html.escape(capture_rel)}" alt="{html.escape(scene)} capture" />'
            if capture_rel
            else '<div class="empty">Capture missing.</div>'
        )
        rows.append(
            f"""
            <section class="scene">
              <div class="meta">
                <h2>{html.escape(scene)}</h2>
                <p><strong>JSX source:</strong> <code>{html.escape(jsx)}</code></p>
                <p><strong>HTML export:</strong> <code>/Users/dev/Downloads/Kassiber.html</code> <span class="tag">{html.escape(html_scene)}</span></p>
              </div>
              <div class="grid">
                <div class="panel">
                  <h3>Reference Image</h3>
                  {reference_panel}
                </div>
                <div class="panel">
                  <h3>Current QML Capture</h3>
                  {capture_panel}
                </div>
              </div>
            </section>
            """
        )

    html_path = output_dir / "index.html"
    html_path.write_text(
        f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Kassiber UI Review Gallery</title>
  <style>
    :root {{
      color-scheme: light;
      --bg: #f0e8db;
      --paper: #f7f1e5;
      --ink: #1a1613;
      --ink-2: #534a40;
      --line: #d9cfbc;
      --accent: #8a1f2b;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      padding: 32px;
      background: var(--bg);
      color: var(--ink);
      font-family: "Helvetica Neue", Helvetica, Arial, sans-serif;
    }}
    h1, h2, h3 {{
      margin: 0;
      font-family: Baskerville, Georgia, serif;
      font-weight: 500;
    }}
    .header {{
      margin-bottom: 28px;
    }}
    .header p, .meta p {{
      margin: 8px 0 0;
      color: var(--ink-2);
      font-size: 14px;
      line-height: 1.5;
    }}
    .scene {{
      margin-bottom: 28px;
      padding: 20px;
      border: 1px solid var(--line);
      background: var(--paper);
    }}
    .meta {{
      margin-bottom: 16px;
    }}
    .tag {{
      display: inline-block;
      margin-left: 8px;
      padding: 2px 8px;
      border: 1px solid var(--line);
      border-radius: 999px;
      font-size: 11px;
      color: var(--ink-2);
    }}
    .grid {{
      display: grid;
      gap: 16px;
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }}
    .panel {{
      border: 1px solid var(--line);
      background: #fffdf8;
      padding: 14px;
    }}
    .panel h3 {{
      margin-bottom: 12px;
      font-size: 20px;
    }}
    .panel img {{
      display: block;
      width: 100%;
      height: auto;
      border: 1px solid var(--line);
      background: white;
    }}
    .empty {{
      min-height: 240px;
      display: flex;
      align-items: center;
      justify-content: center;
      border: 1px dashed var(--line);
      padding: 18px;
      color: var(--ink-2);
      text-align: center;
      line-height: 1.5;
    }}
    code {{
      font-family: Menlo, Monaco, monospace;
      font-size: 12px;
    }}
    @media (max-width: 1100px) {{
      .grid {{
        grid-template-columns: 1fr;
      }}
    }}
  </style>
</head>
<body>
  <header class="header">
    <h1>Kassiber UI Review Gallery</h1>
    <p>This is the screenshot comparison loop for the Claude export. Drop named screenshots into the reference directory and rerun this script to compare them against fresh QML captures automatically.</p>
  </header>
  {''.join(rows)}
</body>
</html>
""",
        encoding="utf-8",
    )
    (output_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Build a side-by-side gallery for Kassiber UI review.")
    parser.add_argument(
        "--reference-images-dir",
        default="/Users/dev/Github/kassiber/temp",
        help="Directory containing named scene screenshots such as welcome.png or overview-data.png.",
    )
    parser.add_argument(
        "--output-dir",
        default="docs/design/review",
        help="Directory where the gallery, manifest, and captures should be written.",
    )
    parser.add_argument(
        "--skip-capture",
        action="store_true",
        help="Reuse existing captures instead of running a fresh preview capture pass.",
    )
    parser.add_argument(
        "--scenes",
        nargs="+",
        default=list(SCENE_SOURCES.keys()),
        help="Scene ids to include in the review gallery.",
    )
    args = parser.parse_args()

    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    captures_dir = output_dir / "generated"
    captures_dir.mkdir(parents=True, exist_ok=True)

    scenes = [scene for scene in args.scenes if scene in SCENE_SOURCES]
    if not args.skip_capture:
        _capture_previews(captures_dir, scenes)
    reference_images = _copy_reference_images(Path(args.reference_images_dir).resolve(), output_dir)
    _render_gallery(output_dir, captures_dir, reference_images, scenes)
    print(f"gallery: {output_dir / 'index.html'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


DEFAULT_SCENES = [
    "welcome",
    "overview-empty",
    "overview-data",
    "transactions",
    "tax",
    "connection-detail",
    "settings",
]


def _run_many(args: argparse.Namespace) -> int:
    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    for scene in args.scenes:
        cmd = [
            sys.executable,
            __file__,
            "--scene",
            scene,
            "--output-dir",
            str(output_dir),
            "--width",
            str(args.width),
            "--height",
            str(args.height),
            "--delay-ms",
            str(args.delay_ms),
        ]
        if args.data_root:
            cmd.extend(["--data-root", str(args.data_root)])
        if args.env_file:
            cmd.extend(["--env-file", str(args.env_file)])
        result = subprocess.run(cmd)
        if result.returncode != 0:
            return result.returncode
    return 0


def _capture_one(args: argparse.Namespace) -> int:
    os.environ["QT_QPA_PLATFORM"] = "offscreen"
    os.environ["KASSIBER_UI_PREVIEW_PAGE"] = args.scene
    os.environ["KASSIBER_UI_CAPTURE"] = "1"
    os.environ["KASSIBER_UI_DISABLE_STATE_WRITE"] = "1"

    from PySide6.QtCore import QTimer

    from kassiber.backends import load_runtime_config, merge_db_backends
    from kassiber.core.runtime import ensure_runtime_layout, resolve_runtime_paths
    from kassiber.db import open_db
    from kassiber.ui.app import build_application

    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = output_dir / f"{args.scene}.png"

    paths = ensure_runtime_layout(resolve_runtime_paths(args.data_root, args.env_file))
    runtime_config = load_runtime_config(paths.env_file)
    conn = open_db(paths.data_root)
    merge_db_backends(conn, runtime_config)
    app, engine, window = build_application(conn, paths.data_root, runtime_config)
    window.setProperty("width", args.width)
    window.setProperty("height", args.height)

    def do_grab() -> None:
        image = window.grabWindow()
        ok = image.save(str(output_path))
        print(f"{args.scene}: saved={ok} path={output_path}")
        window.close()
        conn.close()
        engine.deleteLater()
        app.quit()

    QTimer.singleShot(args.delay_ms, do_grab)
    app.exec()
    return 0 if output_path.exists() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Capture Kassiber UI preview screenshots.")
    parser.add_argument("--scene", help="Single preview scene to capture.")
    parser.add_argument(
        "--scenes",
        nargs="+",
        default=DEFAULT_SCENES,
        help="Preview scenes to capture when --scene is omitted.",
    )
    parser.add_argument("--output-dir", default="docs/design/generated", help="Directory to write PNG files into.")
    parser.add_argument("--width", type=int, default=1360, help="Capture width in pixels.")
    parser.add_argument("--height", type=int, default=860, help="Capture height in pixels.")
    parser.add_argument("--delay-ms", type=int, default=250, help="Delay before capture.")
    parser.add_argument("--data-root", default=None, help="Optional data root to capture from.")
    parser.add_argument("--env-file", default=None, help="Optional env file to capture from.")
    args = parser.parse_args()

    if args.scene:
        return _capture_one(args)
    return _run_many(args)


if __name__ == "__main__":
    raise SystemExit(main())
